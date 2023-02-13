#[cfg(windows)]
fn main() {
    panic!("Confinuum does not support Windows.");
}

#[cfg(not(windows))]
fn main() {}
