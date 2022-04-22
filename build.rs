use capnp::Error;
use capnpc::CompilerCommand;

fn main() -> Result<(), Error> {
    CompilerCommand::new().file("conmon.capnp").run()
}
