//! Own, dependency-free Rust fixture for editing, Cargo and debugger scenarios.

/// Builds a Unicode greeting.
///
/// ```
/// assert_eq!(codisseu_fixture_library::saudação("Rust 🦀"), "Olá, Rust 🦀!");
/// ```
pub fn saudação(nome: &str) -> String {
    format!("Olá, {nome}!")
}

pub fn total(values: &[i32]) -> i32 {
    values.iter().sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unicode_greeting_and_total() {
        assert_eq!(saudação("ação 🦀"), "Olá, ação 🦀!");
        assert_eq!(total(&[19, 23]), 42);
    }
}
