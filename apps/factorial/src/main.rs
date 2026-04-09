fn main() {
    println!("Factorial demo on VyomaOS");
    for n in [0, 1, 5, 7, 10] {
        println!("factorial({n})            = {}", factorial(n));
        println!("factorial_iterative({n}) = {}", factorial_iterative(n));
    }
    println!("double_factorial(5)     = {}", double_factorial(5));
    println!("rising_factorial(3, 4)  = {}", rising_factorial(3, 4));
    println!("combinations(5, 2)      = {}", combinations(5, 2));

    let sum = factorial(5)
        + factorial_iterative(5)
        + double_factorial(5)
        + rising_factorial(3, 4)
        + combinations(5, 2);
    assert_eq!(sum, 625, "verification sum mismatch");
    println!("All assertions passed.");
}

fn factorial(n: i32) -> i32 {
    if n <= 1 { 1 } else { n * factorial(n - 1) }
}

fn factorial_iterative(n: i32) -> i32 {
    (2..=n).product()
}

fn double_factorial(n: i32) -> i32 {
    if n <= 1 { 1 } else { n * double_factorial(n - 2) }
}

fn rising_factorial(n: i32, k: i32) -> i32 {
    (0..k).map(|i| n + i).product()
}

fn combinations(n: i32, k: i32) -> i32 {
    if k > n || k < 0 { return 0; }
    let k = k.min(n - k);
    (0..k).fold(1, |acc, i| acc * (n - i) / (i + 1))
}
