fn main() {
    println!("cargo:rerun-if-changed=../../packaging/windows/melos.rc");
    println!("cargo:rerun-if-changed=../../assets/melos.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        embed_resource::compile("../../packaging/windows/melos.rc", embed_resource::NONE)
            .manifest_required()
            .expect("compile Windows application resources");
    }
}
