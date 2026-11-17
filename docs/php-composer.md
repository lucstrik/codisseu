# PHP com Composer

Abra a raiz do projeto PHP e autorize o provider quando confiar na pasta.
Laravel não é requisito. Os namespaces precisam corresponder ao autoload do
composer.json, e bibliotecas externas precisam estar disponíveis em vendor para
serem analisadas. O Codisseu não instala dependências automaticamente.

O rodapé mostra o PHP declarado para o projeto do arquivo ativo, quando há
`config.platform.php`, `.php-version` ou `require.php`. O tooltip identifica a
origem; requisitos como `^8.2` conservam o intervalo. Salve a configuração para
atualizar o indicador (até3s mais I/O). Sem declaração utilizável, o rodapé
mantém PHP. Essa indicação não configura nem mede o interpretador em execução.

- **Ctrl+Space** ou uma pausa na digitação: sugestões de classes, funções e
  métodos. Aceitar uma classe pode inserir o `use` retornado pelo Phpactor.
  Imports e completion entram juntos no Undo, sem salvar automaticamente.
- **Ctrl+B**, **F12** ou **Ctrl+clique**: definição. **Ctrl+Alt+Esquerda/Direita**:
  histórico de navegação.
- Atributos PHP `#[…]`: completion de classes declaradas como `Attribute` nos
  caminhos Composer conhecidos. Quando essa sugestão vem do adaptador estático,
  insere o nome plenamente qualificado para evitar colisões.
- **Ctrl+J**: live templates. `pubf`, `pubsf`, `prof`, `prif`, `fore` e `eco`
  seguidos de **Tab** expandem construções PHP; Tab/Shift+Tab percorrem campos,
  Escape encerra e Ctrl+Z desfaz.
- **Ctrl+P**: parâmetros da chamada; hover/documentação, referências, formatação
  e Rename continuam nos comandos e contratos existentes do editor.

O corpus novo sem Laravel verifica PSR-4, classmap, autoload.files e autoload-dev
com Phpactor2026.10.07.0, incluindo imports e navegação de classe/função/método/
atributo. Isso não certifica todos os layouts Composer, inspeções do PhpStorm,
atributos/flags ou versões PHP. A referência local e os limites estão no
relatório e no contrato.

Em26/10/2026, a busca complementar de atributos passou a resolver imports
agrupados e namespaces braced/múltiplos por escopo lexical, incluindo aliases
diferentes em cada namespace. Essa certificação aplica-se ao fallback de
atributos; não equivale a todas as inspeções de atributos do PhpStorm.
