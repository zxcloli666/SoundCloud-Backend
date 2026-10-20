fn main() {
    let files = ["proto/inner.proto", "proto/envelope.proto"];
    for file in files {
        println!("cargo:rerun-if-changed={file}");
    }
    let descriptors = protox::compile(files, ["proto"]).expect("chat protos compile");
    prost_build::Config::new()
        .boxed(".scd.chat.v1.Suggest.track")
        .compile_fds(descriptors)
        .expect("chat protos generate");
}
