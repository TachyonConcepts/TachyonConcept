pub mod faf_helpers;
pub mod http;
pub mod kernel;
pub mod memory;
pub mod shift;
pub mod trim;

pub fn print_as_const(bytes: &[u8], name: &str) {
    print!("const {}: &[u8] = &[\n    ", name);
    for (i, b) in bytes.iter().enumerate() {
        print!("{:3},", b);
        if (i + 1) % 16 == 0 {
            print!("\n    ");
        }
    }
    println!("\n];");
}