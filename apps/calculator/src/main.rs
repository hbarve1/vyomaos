fn main() {
    println!("Calculator running on VyomaOS");
    println!("add(25.5, 14.3)       = {:.2}", add(25.5, 14.3));
    println!("subtract(100.0, 35.7) = {:.2}", subtract(100.0, 35.7));
    println!("multiply(7.5, 8.0)    = {:.2}", multiply(7.5, 8.0));
    println!("divide(144.0, 12.0)   = {:.2}", divide(144.0, 12.0));
    println!("divide(10.0, 0.0)     = {} (NaN expected)", divide(10.0, 0.0));
    println!("power(2.0, 8.0)       = {:.0}", power(2.0, 8.0));
    println!("sqrt(144.0)           = {:.2}", sqrt_val(144.0));
}

fn add(a: f64, b: f64) -> f64 { a + b }
fn subtract(a: f64, b: f64) -> f64 { a - b }
fn multiply(a: f64, b: f64) -> f64 { a * b }
fn divide(a: f64, b: f64) -> f64 {
    if b != 0.0 { a / b } else { f64::NAN }
}
fn power(base: f64, exp: f64) -> f64 { base.powf(exp) }
fn sqrt_val(x: f64) -> f64 { x.sqrt() }
