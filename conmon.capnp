@0xa074fbab61132cbd;

interface Conmon {
    exec @0 (stdout: Stdio, stderr: Stdio) -> (stdin: Stdio);

    interface Stdio {
        send @0 (data: Data) -> ();
    }
}
