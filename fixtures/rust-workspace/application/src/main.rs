use codisseu_fixture_library::{saudação, total};

fn main() {
    let valores = [19, 23];
    let resposta = total(&valores);
    println!("{} total={resposta}", saudação("Codisseu 🦀"));
    for (index, argument) in std::env::args().skip(1).enumerate() {
        println!("arg[{index}]={argument}");
    }
}
