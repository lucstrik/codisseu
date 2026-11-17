# Fixture Rust própria

Workspace Cargo de dois membros: biblioteca com identificador/documentação
Unicode, teste unitário e doctest; aplicação com locals determinísticos e teste
real de argumentos. Sem dependências de rede, build.rs ou proc macros.

Os testes do Codisseu copiam estes arquivos para pasta temporária fora do
workspace principal e usam CARGO_TARGET_DIR separado. Check/build/run/test/
clippy/doc são explícitos; não gerar target nem regravar o Cargo.lock nesta fonte canônica.
O teste falho de aceitação é acrescentado somente à cópia temporária.
