use crate::conmon_capnp::conmon::{
    stdio::{self, SendParams, SendResults, Server},
    Client,
};
use capnp::capability::Promise;
use capnp_rpc::{pry, rpc_twoparty_capnp, twoparty, RpcSystem};
use futures::{future, AsyncReadExt, TryFutureExt};
use rpc_twoparty_capnp::Side;
use std::{env, error, net::ToSocketAddrs, str};
use tokio::task::LocalSet;
use tokio_util::compat::TokioAsyncReadCompatExt;
use twoparty::VatNetwork;

struct StdoutImpl;
struct StderrImpl;

impl Server for StdoutImpl {
    fn send(&mut self, params: SendParams, _: SendResults) -> Promise<(), capnp::Error> {
        let data = pry!(pry!(params.get()).get_data());
        println!("Got stdout data: {}", unsafe {
            str::from_utf8_unchecked(data)
        });
        Promise::ok(())
    }
}

impl Server for StderrImpl {
    fn send(&mut self, params: SendParams, _: SendResults) -> Promise<(), capnp::Error> {
        let data = pry!(pry!(params.get()).get_data());
        println!("Got stderr data: {}", unsafe {
            str::from_utf8_unchecked(data)
        });
        Promise::ok(())
    }
}

pub async fn main() -> Result<(), Box<dyn error::Error>> {
    let args = env::args().collect::<Vec<_>>();
    if args.len() != 3 {
        println!("usage: {} client HOST:PORT", args[0]);
        return Ok(());
    }

    let addr = args[2]
        .to_socket_addrs()?
        .next()
        .expect("could not parse address");

    LocalSet::new()
        .run_until(async move {
            let stream = tokio::net::TcpStream::connect(&addr).await?;
            stream.set_nodelay(true)?;
            let (reader, writer) = TokioAsyncReadCompatExt::compat(stream).split();
            let rpc_network = Box::new(VatNetwork::new(
                reader,
                writer,
                Side::Client,
                Default::default(),
            ));
            let mut rpc_system = RpcSystem::new(rpc_network, None);

            let conmon: Client = rpc_system.bootstrap(Side::Server);

            let mut request = conmon.exec_request();

            let stdout_client = capnp_rpc::new_client(StdoutImpl);
            request.get().set_stdout(stdout_client);

            let stderr_client = capnp_rpc::new_client(StderrImpl);
            request.get().set_stderr(stderr_client);

            let future = request.send().promise.and_then(|response| {
                let stdin_client: stdio::Client = response.get().unwrap().get_stdin().unwrap();
                let mut stdin_request = stdin_client.send_request();
                stdin_request.get().set_data(b"Hello stdin");
                stdin_request.send().promise
            });

            future::try_join(rpc_system, future).await?;
            Ok(())
        })
        .await
}
