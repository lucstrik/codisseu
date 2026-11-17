# Adaptadores externos de debug

O Codisseu executa adaptadores abertos independentes, sem carregar VS Code, PhpStorm/WebStorm ou código proprietário. As versões e a identidade exata dos arquivos estão no [manifesto](../../../tools/debug-adapters.json), que também registra os pacotes nomeados do PHP VSIX e hashes dos avisos distribuídos.

- [xdebug/vscode-php-debug 1.40.2](https://github.com/xdebug/vscode-php-debug/tree/v1.40.2): [MIT](licenses/php-LICENSE.txt). O VSIX distribui runtime/Node modules; seus LICENSE/notices originais são preservados no destino `.tools/debug/php/extension`.
- [microsoft/vscode-js-debug 1.140.0](https://github.com/microsoft/vscode-js-debug/tree/v1.140.0): [MIT](licenses/javascript-LICENSE.txt) e [avisos compilados do entrypoint](licenses/javascript-bundled-notices.txt), preservados literalmente. O tarball standalone original e seus arquivos continuam íntegros em `.tools/debug`.

Instalação explícita com `python3 tools/install_debug_adapters.py`; nenhuma instalação automática ao abrir pasta/configuração. `--verify` é somente leitura. Download/hash/extract não executam scripts dos pacotes. Ícones/logotipos incluídos na distribuição externa não são incorporados ao desenho da interface do Codisseu.

Este inventário fixa estas distribuições; uma troca de versão requer nova auditoria de fonte/distribuição/avisos e testes reais. A presença de uma licença pública não substitui a observação funcional dos entrypoints fora do host VS Code; os cenários estão em `crates/codisseu-debug/tests/real_debug.rs` e no [contrato do backend](../../../docs/m5-debug-backend.md).
