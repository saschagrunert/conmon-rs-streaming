use crate::conmon_capnp::conmon::{
    self,
    stdio::{self, SendParams, SendResults},
    Client, ExecParams, ExecResults,
};
use capnp::capability::Promise;
use capnp_rpc::{pry, rpc_twoparty_capnp, twoparty, RpcSystem};
use futures::{channel::mpsc, future, AsyncReadExt, FutureExt, StreamExt};
use rpc_twoparty_capnp::Side;
use std::{
    cell::RefCell, collections::HashMap, env, error, net::ToSocketAddrs, rc::Rc, str, thread,
    time::Duration,
};
use tokio::{
    net::TcpListener,
    task::{self, LocalSet},
};
use tokio_util::compat::TokioAsyncReadCompatExt;
use twoparty::VatNetwork;

struct ExecHandle {
    stdout_client: stdio::Client,
    stderr_client: stdio::Client,
}

struct ExecMap {
    execs: HashMap<u64, ExecHandle>,
}

impl ExecMap {
    fn new() -> ExecMap {
        ExecMap {
            execs: HashMap::new(),
        }
    }
}

struct StdinImpl {}

impl stdio::Server for StdinImpl {
    fn send(&mut self, params: SendParams, _: SendResults) -> Promise<(), capnp::Error> {
        let data = pry!(pry!(params.get()).get_data());
        println!("Got stdin data: {}", unsafe {
            str::from_utf8_unchecked(data)
        });
        Promise::ok(())
    }
}

struct Server {
    next_id: u64,
    execs: Rc<RefCell<ExecMap>>,
}

impl Server {
    fn new() -> (Server, Rc<RefCell<ExecMap>>) {
        let execs = Rc::new(RefCell::new(ExecMap::new()));
        (
            Server {
                next_id: 0,
                execs: execs.clone(),
            },
            execs.clone(),
        )
    }
}

impl conmon::Server for Server {
    fn exec(&mut self, params: ExecParams, mut results: ExecResults) -> Promise<(), capnp::Error> {
        println!("New exec request");

        self.execs.borrow_mut().execs.insert(
            self.next_id,
            ExecHandle {
                stdout_client: pry!(pry!(params.get()).get_stdout()),
                stderr_client: pry!(pry!(params.get()).get_stderr()),
            },
        );

        results.get().set_stdin(capnp_rpc::new_client(StdinImpl {}));

        self.next_id += 1;
        Promise::ok(())
    }
}

pub async fn main() -> Result<(), Box<dyn error::Error>> {
    let args = env::args().collect::<Vec<_>>();
    if args.len() != 3 {
        println!("usage: {} server HOST:PORT", args[0]);
        return Ok(());
    }

    let addr = args[2]
        .to_socket_addrs()?
        .next()
        .expect("could not parse address");

    LocalSet::new()
        .run_until(async move {
            let listener = TcpListener::bind(&addr).await?;
            let (server, execs) = Server::new();
            let conmon: Client = capnp_rpc::new_client(server);

            let handle_incoming = async move {
                loop {
                    let (stream, _) = listener.accept().await?;
                    stream.set_nodelay(true)?;

                    let (reader, writer) = TokioAsyncReadCompatExt::compat(stream).split();
                    let network = VatNetwork::new(reader, writer, Side::Server, Default::default());

                    let rpc_system = RpcSystem::new(Box::new(network), Some(conmon.clone().client));
                    task::spawn_local(Box::pin(rpc_system.map(|_| ())));
                }
            };

            let (tx, mut rx) = mpsc::unbounded::<()>();
            thread::spawn(move || {
                while let Ok(()) = tx.unbounded_send(()) {
                    thread::sleep(Duration::from_secs(1));
                }
            });

            let send_to_stdout_stderr = async move {
                while let Some(()) = rx.next().await {
                    let execs = &mut execs.borrow_mut().execs;

                    for (_, exec) in execs.iter_mut() {
                        let mut stdout_request = exec.stdout_client.send_request();
                        stdout_request.get().set_data(b"Hello stdout");
                        task::spawn_local(stdout_request.send().promise);

                        let mut stderr_request = exec.stderr_client.send_request();
                        stderr_request.get().set_data(b"Hello stderr");
                        task::spawn_local(stderr_request.send().promise);
                    }
                }
                Ok::<(), Box<dyn error::Error>>(())
            };

            let _: ((), ()) = future::try_join(handle_incoming, send_to_stdout_stderr).await?;
            Ok(())
        })
        .await
}
