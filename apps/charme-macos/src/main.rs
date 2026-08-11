fn main() {
    #[cfg(target_os = "macos")]
    println!("Charme macOS frontend has not been implemented yet.");

    #[cfg(not(target_os = "macos"))]
    println!("This package is the macOS frontend for Charme.");
}
