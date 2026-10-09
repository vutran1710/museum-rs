//! The executable both example registries publish: prints which target it was built for.

fn main() {
    println!("hello from the museum, built for {}", museum::HOST_TARGET);
}
