# Laravel e arquivos do projeto

Abra a pasta que contém `composer.json`. Para analisar classes do framework,
as dependências precisam estar instaladas em `vendor` e os namespaces do projeto
precisam estar declarados no autoload PSR-4. Autorize os providers somente quando
confiar no projeto. A primeira indexação pode demorar mais que as seguintes.
O Codisseu não instala dependências nem inicia Artisan ou o banco automaticamente.

Ao digitar código PHP, JS ou TS, as sugestões aparecem após uma breve pausa.
**Ctrl+Space** também solicita sugestões; **Enter** insere a opção selecionada e
**Tab** substitui o prefixo conforme o completion existente. **F12** ou **Ctrl+B** abre a
classe ou o método; **Ctrl+Alt+Esquerda** volta ao ponto anterior. As sugestões são
calculadas sobre a versão atual do documento, inclusive antes de salvar.

No PHP, digite `lroute`, `lcontroller`, `lmodel`, `lmigration`, `lvalidate`,
`lview`, `lbelongs`, `lhasmany`, `lscope` ou `ltransaction` e pressione **Tab**.
**Ctrl+J** abre a lista de templates aplicáveis. Em `.blade.php`, há templates
para `@if`, `@foreach`, `@forelse`, `@section`, `@extends`, `@include`, `@auth`,
`@can`, `@error`, `@csrf` e `@method`. **Tab** e **Shift+Tab** percorrem os campos;
**Escape** encerra; **Ctrl+Z** desfaz. Somente **Ctrl+S** grava o resultado.

Na árvore Project, use os ícones **New File/New Folder** ou o menu do botão
direito, inclusive na área vazia. Enter confirma o nome; Escape cancela. Caminhos
como `app/Http/Controllers` criam pastas aninhadas. Selecione um arquivo na árvore
e pressione **F2**, ou escolha **Rename…**, para alterar seu nome/extensão.
Essa ação preserva alterações abertas, mas não reescreve referências de código;
a refatoração semântica do editor continua disponível separadamente.
**Delete** pede confirmação para mover o item à Lixeira do sistema. Arquivos com
alterações não salvas precisam ser salvos ou fechados antes da exclusão.

A validação Laravel desta etapa usa **12.70.1**, com classes PSR-4, métodos do
projeto, herdados, Request, Route e chamadas Eloquent delimitadas. `Model::where`
é complementado por leitura das declarações reais do framework quando o provider
não encontra resultado; a descrição identifica essa inferência. Não há promessa
de suporte a todas as versões, macros e builders personalizados, colunas vindas
do banco, container dinâmico ou análise semântica completa de Blade. Se instalar
as dependências depois de iniciar o provider e os símbolos continuarem ausentes,
pare e inicie os providers pelas configurações para renovar a indexação.

## Recursos do Laravel no editor

A extensão M3.29 acrescenta nomes reais do projeto ao mesmo popup de sugestões:

| Contexto PHP/Blade | Conteúdo e destino |
|---|---|
| `view('…')`, `View::make/exists`, `@extends`, `@include`, `@includeIf`, `@component`, `@each` | Arquivos em resources/views, incluindo subpastas por ponto |
| `config('…')`, `Config::get/has/set` | Chaves literais de config/*.php, incluindo arrays aninhados |
| `__('…')`, `trans`, `trans_choice`, `Lang::get/has/choice`, `@lang`, `@choice` | Chaves de lang/resources/lang e arquivos JSON de idioma |
| `route('…')`, `to_route`, `Route::has` | Nomes literais de rotas, incluindo prefixos estáticos de grupos |
| `asset('…')`, `secure_asset` | Arquivos existentes sob public |
| `[Controller::class, '…']` em rotas | Métodos públicos declarados no controlador PSR-4 e seus pais locais |

Use **Ctrl+Space** para completar. Use **Ctrl+B**, **F12** ou **Ctrl+clique**
para abrir o destino, e **Ctrl+Alt+Esquerda** para voltar. Traduções com vários
idiomas e nomes de rota duplicados podem abrir o seletor existente de destinos.
As aspas são preservadas e nomes com pontos/barras são substituídos por inteiro,
inclusive quando há um sufixo depois do cursor. Undo desfaz a aceitação; Save
continua explícito.

Em Blade, Ctrl+Space também completa diretivas como `@foreach`. Depois de
completar o nome, Tab expande o template correspondente. O catálogo agora tem
**47 templates próprios** PHP/Blade; Ctrl+J permite escolher sem memorizar todos.
Novos exemplos: `pubf`, `pubsf`, `prof`, `prif`, `fore`, `eco`, `lrequest`,
`lresource`, `lcommand`, `ljob`, `lpolicy`, `lhasone`, `lbelongsmany`, `lmorphmany`,
`ltest`, `@unless`, `@isset`, `@empty`, `@switch`, `@push`, `@stack`, `@props`,
`@slot`, `@once`, `@php` e `@vite`. Os corpos são exemplos editáveis; não
adivinham imports/nomes do projeto nem certificam todas as versões Laravel.

Ao abrir uma pasta com vários projetos, os recursos acima usam o composer.json
mais próximo do arquivo para separar os serviços. A verificação de38 projetos
reais foi estática e não executou aplicações. A validação completa de provider
Laravel desta extensão continua usando a fixture própria Laravel12.70.1.

Esta resolução lê fontes e arquivos como dados, sem Artisan, Composer install,
.env ou banco. Rotas calculadas, resources automáticos, namespaces de views
registrados em runtime, JSON/arrays calculados, includes dinâmicos, configuração
de caminhos customizados e o restante de Laravel Idea permanecem fora deste
corpus. Se o inventário exceder o orçamento, completion informa truncamento e a
navegação recusa resultado parcial. Assets binários dependem do suporte existente
para abrir o tipo de arquivo. Contrato.

Os recursos comuns de PHP/Composer também funcionam sem Laravel; consulte
[PHP com Composer](php-composer.md).

## Ampliação de26/10/2026

Ctrl+Space e F12/Ctrl+B passam também pelos adaptadores estáticos de scopes,
builders/campos/relações Eloquent, campos de FormRequest e validação, componentes
Blade, wire:model/ações Livewire e nomes de página em Inertia::render. Os destinos
são declarações existentes; os mesmos Undo e histórico do editor são usados.
Sugestões Eloquent e de propriedades request complementam os membros PHP.

Para campos request, declare o tipo do parâmetro e mantenha rules como array
literal. Para recursos de Livewire/Inertia, a dependência precisa constar do
Composer/metadata instalado e o arquivo seguir a convenção coberta. Isso não
instala dependências nem executa a aplicação. Slots Blade são candidatos
inferidos, identificados no popup. Resultados calculados em runtime podem
continuar sem sugestão; veja a matriz php-laravel-coverage.md para limites.
