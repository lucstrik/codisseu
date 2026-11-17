# Testes PHP no Codisseu

Abra um projeto que já tenha PHPUnit ou Pest instalado pelo Composer.
**Executar → Testes PHP · PHPUnit/Pest…** abre a configuração; abrir a janela
não executa PHP. O resultado aparece na aba **Run** do painel inferior.

- Framework Automático prefere vendor/bin/pest, depois vendor/bin/phpunit.
  Selecione PHPUnit/Pest explicitamente quando ambos estiverem no projeto.
- Interpretador vazio usa o PHP privado no pacote, ou PHP do PATH no checkout.
  Um caminho explícito seleciona a versão/extensões que sua aplicação exige.
- Runner vazio usa vendor/bin. Um runner PHP/PHAR e uma configuração XML podem
  ser indicados por caminho absoluto ou relativo dentro do projeto.
- XML vazio deixa a descoberta de phpunit.xml/phpunit.xml.dist ao framework.
  Não executamos composer install, scripts npm ou instalação automática.
- O prazo padrão é300segundos, configurável de1 a3600. Configurações ficam no
  perfil externo da IDE, no arquivo irmão chamado session.php-tests.json.

Use **Executar suite** ou **Executar arquivo ativo**. O menu Executar e o contexto
no editor PHP também oferecem execução do arquivo. O primeiro uso pode solicitar
revisão de configuração; abrir um arquivo PHP comum não o transforma em teste.
Salve seus buffers antes de executar: a IDE recusa testar uma versão diferente
do editor e nunca salva implicitamente.

A árvore representa os casos reais do JUnit da execução. Selecione um teste
para ler o motivo da falha, stack e duração. O link de arquivo:linha e o contexto
**Abrir fonte do teste** usam o editor compartilhado; duplo clique também navega.
Ausência de linha no relatório não vira uma coordenada de declaração inventada.
Show Passed/Show Ignored filtram a árvore; o menu de opções ordena por nome ou
duração. Raiz/classe/datasets expandem e recolhem, com tempos e estados dos filhos.

**Run selected test** executa somente o caso selecionado, inclusive um dataset.
**Rerun Failed Tests** executa somente falhas/erros da última execução, agrupados
por arquivo. **Rerun / Ctrl+F5** repete o mesmo escopo com framework/runner resolvidos.
**Stop / Ctrl+F2** interrompe e recolhe o grupo próprio no Unix. Cancelamento,
timeout, runner ausente, relatório inválido e zero testes nunca aparecem como sucesso.
Console da execução mostra stdout/stderr inertes; Copiar saída preserva o texto
retido. Saída limita1MiB, JUnit16MiB/10000casos e detalhes64KiB por caso.

Linux/Unix é a execução implementada; Windows exige tratamento equivalente de
processos antes de liberar o runner. Não há cobertura, watch, paralelismo ou debug
de teste neste incremento. A árvore inicial é obtida após executar suite/arquivo;
não há descoberta antecipada de casos. Nem toda versão/plugin do framework foi
certificado; versões e cenários efetivamente testados estão em docs/status.md.
Referência visual e limites de comparação: php-tests-reference.md.
