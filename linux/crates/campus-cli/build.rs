#[path = "src/args.rs"]
mod args;
use clap::CommandFactory;
fn main() {
    if std::env::var("PROFILE").as_deref() == Ok("release")
        && std::env::var_os("CARGO_FEATURE_DEVELOPER").is_some()
    {
        panic!("developer capture/test clock must not be enabled in release builds");
    }
    let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    let mut cmd = args::Args::command();
    for shell in [
        clap_complete::Shell::Bash,
        clap_complete::Shell::Zsh,
        clap_complete::Shell::Fish,
    ] {
        clap_complete::generate_to(shell, &mut cmd, "campus", &out).unwrap();
    }
    let man = clap_mangen::Man::new(args::Args::command());
    let mut file = std::fs::File::create(out.join("campus.1")).unwrap();
    man.render(&mut file).unwrap();
    println!("cargo:rerun-if-changed=src/args.rs");
}
