# Segurança

O Codisseu está em alpha. As correções se concentram na versão mais recente;
não há manutenção separada das versões antigas nesta fase.

## Como relatar uma falha

Se o problema envolver execução de código sem autorização, acesso a arquivos
fora do escopo permitido, vazamento de credenciais ou perda de dados causada
por uma falha de segurança, prefira um relato privado.

Na página do repositório, procure **Security → Report a vulnerability**. Se essa
opção não estiver disponível, abra uma issue apenas pedindo um canal privado,
sem publicar os detalhes da falha, a prova de conceito ou qualquer segredo.

No relato privado, inclua a versão, o sistema, as condições necessárias para
reproduzir e um exemplo mínimo com dados fictícios. Não envie senhas, tokens,
chaves, dumps de banco ou arquivos de terceiros.

## Ao testar

Use um projeto e um banco descartáveis, sem atingir serviços ou dados de outras
pessoas. A confiança em projetos, as permissões do AI Chat e as confirmações de
operações de banco fazem parte do fluxo do produto; descreva quando uma dessas
proteções puder ser contornada.

Uma falha comum de interface, instalação ou ferramenta pode ser reportada nas
[issues](https://github.com/lucstrik/codisseu/issues/new/choose), com os dados
sensíveis removidos.
