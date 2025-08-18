/// Basic arithmetic operations for WebAssembly

#[no_mangle]
pub extern "C" fn add(a: f64, b: f64) -> f64 {
    a + b
}

#[no_mangle]
pub extern "C" fn subtract(a: f64, b: f64) -> f64 {
    a - b
}

#[no_mangle]
pub extern "C" fn multiply(a: f64, b: f64) -> f64 {
    a * b
}

#[no_mangle]
pub extern "C" fn divide(a: f64, b: f64) -> f64 {
    if b != 0.0 {
        a / b
    } else {
        f64::NAN
    }
}

#[no_mangle]
pub extern "C" fn power(base: f64, exponent: f64) -> f64 {
    base.powf(exponent)
}

#[no_mangle]
pub extern "C" fn sqrt(x: f64) -> f64 {
    x.sqrt()
}

#[no_mangle]
pub extern "C" fn sin(x: f64) -> f64 {
    x.sin()
}

#[no_mangle]
pub extern "C" fn cos(x: f64) -> f64 {
    x.cos()
}

#[no_mangle]
pub extern "C" fn log(x: f64) -> f64 {
    x.ln()
}
