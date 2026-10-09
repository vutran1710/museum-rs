//! Captures Cargo's exact target triple, which `std::env::consts` cannot tell apart (msvc vs gnu).

fn main() -> Result<(), std::env::VarError> {
    println!("cargo:rustc-env=HOST_TARGET={}", std::env::var("TARGET")?);
    Ok(())
}
