# Amorphis — Documento de direção do projeto

> **Estado:** especificação inicial / direção de implementação. Não é uma declaração de funcionalidades já implementadas.
>
> **Objetivo:** construir um *screen locker* pequeno, fiável e seguro, inspirado na filosofia do `slock`, com fundo baseado numa captura do ambiente de trabalho sujeita a **blur** e/ou **pixelização**, e suporte real a **X11 e Wayland**.

## 1. Intenção e filosofia

O Amorphis nasceu de uma conversa sobre um locker com a simplicidade do `slock` e os efeitos visuais da patch de blur/pixelização do `slock`. O nome é **Amorphis**.

O programa deve fazer uma coisa e fazê-la bem: **bloquear a sessão e permitir desbloqueá-la mediante autenticação**. A estética não pode ser confundida com segurança. Não queremos transformar o locker numa shell, barra, centro de controlo ou plataforma de widgets.

Princípios:

- **Minimalismo funcional:** interface reduzida ao necessário, poucas dependências justificadas, comportamento previsível.
- **Separação de responsabilidades:** bloqueio, autenticação, captura/efeitos e apresentação visual são partes distintas.
- **Suporte genuíno a X11 e Wayland:** não simular segurança desenhando uma janela sobre o ecrã.
- **Sem preferências pessoais codificadas à força:** não assumir compositor, terminal, distribuição ou ferramentas externas específicas do autor.
- **Segurança primeiro:** qualquer falha relevante deve manter a sessão protegida, nunca revelar o ambiente de trabalho por conveniência estética.
- **Implementação auditável:** documentação clara, decisões justificadas e código suficientemente simples para revisão.

## 2. Experiência pretendida

Ao ativar o Amorphis:

1. A sessão entra num estado de bloqueio efetivo, usando o mecanismo correto para o backend em uso.
2. É apresentada uma imagem do ambiente de trabalho imediatamente anterior ao bloqueio, com **desfoque (blur)**, **pixelização**, ou a combinação escolhida.
3. O utilizador introduz a palavra-passe, com feedback visual discreto e sem exposição dos caracteres.
4. A autenticação é verificada por **PAM**.
5. Apenas após autenticação bem-sucedida o bloqueio é removido.

O efeito de fundo é uma escolha **visual**, não um mecanismo de ocultação seguro dos dados da sessão. O desenho deve evitar qualquer instante em que o ecrã fique desprotegido. Quando não for possível obter uma captura segura ou apropriada, usar um fundo opaco simples e **continuar a bloquear**.

A especificação não exige relógio, notificações, leitor de música, dashboard, widgets, plugins ou animações elaboradas. Não acrescentar estas funcionalidades sem nova decisão explícita.

## 3. Arquitetura pretendida

A inspiração arquitetural é a ideia de um único programa com dois backends reais, sem forçar uma implementação artificialmente idêntica onde os protocolos divergem.

```text
                 Amorphis
                    |
          Coordenação / estado
          /         |          \
 Autenticação   Visualização   Bloqueio efetivo
     PAM        + efeitos       /          \
                             X11         Wayland
```

### 3.1. Núcleo partilhado

Responsável por:

- ciclo de vida do locker e estados de bloqueio/autenticação;
- tratamento de entrada sem guardar ou registar palavras-passe desnecessariamente;
- configuração mínima dos efeitos visuais;
- processamento de imagem em memória quando viável;
- gestão de erros, limpeza de recursos e comunicação entre componentes.

A partilha de código é desejável, **não obrigatória** quando introduzir abstrações excessivas ou comprometer segurança.

### 3.2. Backend X11

Investigar o `slock` como referência de funcionamento, especialmente:

- controlo do ecrã e das janelas relevantes;
- *grabs* de teclado e rato e respetiva verificação de sucesso;
- múltiplos monitores e alterações de geometria;
- reação a falhas de *grabs*, perda de foco e erros de servidor.

**Limitação a documentar:** o modelo de segurança de X11 não fornece as mesmas garantias de isolamento de um protocolo de bloqueio do compositor Wayland. O Amorphis deve implementar o melhor bloqueio possível em X11, sem prometer garantias que X11 não pode oferecer.

### 3.3. Backend Wayland

A implementação tem de usar um **protocolo de bloqueio de sessão efetivo**, preferencialmente `ext-session-lock-v1`, quando suportado pelo compositor. Uma superfície `layer-shell`, por si só, **não constitui um bloqueio seguro**.

Investigar e implementar:

- deteção de suporte do protocolo e comportamento perante ausência de suporte;
- criação de superfícies de bloqueio nos outputs aplicáveis;
- tratamento de outputs ligados/desligados, mudanças de escala e resolução;
- sequência correta entre pedido de bloqueio, confirmação do compositor e apresentação;
- perda de ligação ao compositor, encerramento inesperado e recuperação segura.

**Não assumir** que a captura do ecrã está disponível em qualquer compositor Wayland. Estudar os caminhos possíveis (por exemplo, portal de captura ou mecanismos específicos do compositor), respetivas permissões, disponibilidade e implicações de segurança. Uma captura deve ser obtida **antes** de o conteúdo ficar inacessível, sem criar uma janela temporal insegura; se isso não puder ser garantido, usar fundo opaco.

O suporte Wayland deve ser descrito em termos de **protocolos e capacidades verificadas**, não como uma promessa universal para todos os compositores.

## 4. Autenticação e privilégios

Usar **PAM**, respeitando as políticas do sistema. Investigar a forma mais segura de integrar a autenticação e a necessidade eventual de um auxiliar com privilégios, em vez de correr toda a interface com privilégios elevados.

Requisitos:

- nunca escrever palavras-passe em logs, ficheiros temporários ou argumentos de processo;
- limpar buffers sensíveis quando tecnicamente possível;
- não desbloquear em caso de erro, interrupção ou falha de autenticação;
- evitar *deadlocks* e situações em que o utilizador fique sem forma legítima de autenticar;
- documentar claramente configuração PAM, instalação e privilégios;
- considerar limites de tentativas, repetição de teclas, layouts de teclado e comportamento de teclas especiais;
- analisar a superfície de ataque criada por bibliotecas de imagem, captura e renderização.

**Importante:** não copiar cegamente a abordagem de autenticação do `slock`; avaliar a integração PAM adequada à implementação escolhida.

## 5. Blur, pixelização e captura

Há dois materiais de referência que acompanharão o projeto:

1. **Tarball do código-fonte do `slock`** — referência de simplicidade, estrutura e backend X11.
2. **Patch de blur/pixelização do `slock`** — referência visual e algorítmica, não código a aplicar indiscriminadamente.

O agente que implementar deve ler os dois materiais **antes de definir a arquitetura final** e identificar o que pode ser reutilizado, adaptado ou reescrito.

A patch examinada na conversa usa **Imlib2** e inclui processamento de imagem para blur e pixelização. Verificar cuidadosamente a sua correção, sobretudo limites de imagem, blocos parciais nas extremidades, formatos de pixel, uso de memória e limpeza de recursos. A patch é contexto, **não uma especificação de qualidade ou segurança**.

Direção visual:

- fundo derivado da imagem do ambiente de trabalho anterior ao bloqueio;
- blur e pixelização como efeitos principais, ativáveis de forma simples;
- fundo de recurso seguro: imagem estática opcional, com cor sólida como último fallback;
- interface discreta de entrada/autenticação;
- sem dependência obrigatória de ficheiros externos para guardar capturas;
- evitar exposição da captura original durante transições ou falhas.

A implementação concreta dos efeitos (biblioteca, algoritmo, aceleração ou CPU) deve ser decidida após análise de custo, portabilidade e dependências. **Não impor Imlib2** apenas por aparecer na patch.

## 6. Linguagem de programação — decisão deliberadamente em aberto

**Haskell foi sugerido na conversa, mas NÃO é requisito.** Não escolher Haskell por piada, originalidade ou fidelidade ao primeiro impulso.

Pede-se ao modelo/engenheiro que assuma o trabalho que **avalie explicitamente a linguagem mais aconselhada** para este projeto. A avaliação deve considerar:

- qualidade e maturidade das bibliotecas para X11, Wayland e `ext-session-lock-v1`;
- integração com PAM e eventuais componentes privilegiados;
- segurança de memória, FFI e tratamento de recursos;
- tamanho do binário, consumo, tempos de arranque e dependências de runtime;
- portabilidade e facilidade de empacotamento em distribuições diferentes;
- clareza do código, manutenção e capacidade de auditoria;
- facilidade de implementar processamento de imagem e os dois backends sem duplicação desnecessária.

Comparar **Haskell** com alternativas plausíveis, como **C, Rust ou outras que se justifiquem**. Se Haskell se revelar inadequado ou desproporcionadamente complexo, **dizer isso sem rodeios, justificar e recomendar a melhor alternativa**. Não é necessário manter uma linguagem única para todos os componentes se houver um motivo técnico forte para os separar, mas evitar complexidade gratuita.

**Entrega desta fase:** uma recomendação fundamentada da linguagem e das bibliotecas, antes de iniciar implementação extensa.

## 7. Configuração e comportamento

Manter a configuração pequena e previsível. Definir posteriormente, com justificação, se bastam opções de linha de comandos, um ficheiro de configuração simples ou uma combinação das duas.

Configurações candidatas, sujeitas a avaliação:

- selecionar blur, pixelização, combinação ou fundo sólido;
- selecionar uma imagem estática de fundo e o respetivo ajuste (`cover`, `contain` ou `center`), mantendo sempre a cor sólida como último fallback;
- intensidade de blur e tamanho dos blocos de pixelização;
- feedback mínimo de autenticação;
- seleção automática do backend conforme a sessão, com erro claro quando não for suportado.

Não criar um sistema de temas, plugins ou uma linguagem de configuração própria nesta fase. Não codificar nomes de compositor ou ferramentas pessoais como dependências universais.

## 8. Requisitos de robustez e testes

Preparar uma matriz de testes, no mínimo, para:

- bloqueio e desbloqueio corretos em X11;
- bloqueio e desbloqueio em compositores Wayland com suporte efetivo ao protocolo;
- comportamento seguro quando o protocolo Wayland não está disponível;
- palavra-passe incorreta, PAM indisponível ou erro de autenticação;
- falha de captura, imagem inválida e memória insuficiente;
- um e vários monitores, *hotplug*, diferentes resoluções e escalas;
- encerramento inesperado do processo, perda de ligação e sinais;
- tentativas de contornar o bloqueio por foco, teclado, rato ou alterações de outputs;
- funcionamento sem blur/pixelização e sem captura;
- consumo de memória, latência de arranque e dependências reais.

Não afirmar que um locker é seguro apenas porque a imagem de bloqueio cobre o ecrã. Definir critérios de aceitação por backend e registar limitações conhecidas.

## 9. O que fica fora do âmbito inicial

- Shell de desktop, barra, widgets e dashboards.
- Relógio, meteorologia, música, notificações e integração cosmética desnecessária.
- Reescrever o compositor ou inventar um protocolo de bloqueio próprio.
- Dependência obrigatória de um compositor específico.
- Aplicar diretamente uma patch X11 e chamar ao resultado «compatível com Wayland».
- Adicionar dependências pesadas sem benefício demonstrável.
- Confundir efeitos visuais com a garantia de bloqueio da sessão.

## 10. Plano de trabalho pedido a quem pegar no projeto

### Fase A — estudo e proposta (antes de programar)

1. Ler este documento e os dois materiais de referência (`slock` e patch).
2. Inspecionar o código e a patch: versão, licenças, dependências, pontos reutilizáveis e problemas potenciais.
3. Estudar mecanismos reais de bloqueio X11 e Wayland e captura em cada ambiente.
4. Comparar linguagens e bibliotecas; **avaliar Haskell sem o tratar como obrigação**.
5. Propor arquitetura, dependências, fluxos de autenticação e falha segura.
6. Identificar limitações, riscos e decisões ainda por validar com o autor.
7. Apresentar um plano de implementação por etapas e testes de aceitação.

### Fase B — implementação incremental (após aprovação da proposta)

1. Criar a estrutura mínima do projeto e documentação inicial.
2. Implementar e testar o mecanismo de autenticação de forma isolada.
3. Implementar um primeiro backend de bloqueio real, com fundo opaco.
4. Implementar o segundo backend, respeitando as diferenças de protocolo.
5. Introduzir captura e efeitos visuais sem enfraquecer o bloqueio.
6. Testar falhas, múltiplos monitores, diferentes ambientes e privilégios.
7. Rever dependências, empacotamento, documentação e limites de segurança.

A ordem concreta dos backends deve ser proposta e justificada pelo implementador. **Não começar por efeitos visuais enquanto o bloqueio real não estiver sólido.**

## 11. Entregáveis esperados

- análise das referências `slock` e patch, incluindo licenças e eventuais defeitos;
- decisão justificada sobre linguagem, bibliotecas e dependências;
- desenho arquitetural com responsabilidades e fronteiras de segurança;
- plano de implementação incremental;
- matriz de testes e critérios de aceitação;
- documentação de instalação, configuração, suporte e limitações quando houver código.

## 12. Instrução final ao modelo que assumir o trabalho

> **Não inventes requisitos nem comeces por escrever um locker visualmente apelativo que não bloqueia verdadeiramente a sessão.** Lê o `slock` e a patch como material de estudo; identifica o que serve e o que não serve. Preserva a filosofia minimalista, implementa autenticação PAM e bloqueio efetivo com caminhos distintos para X11 e Wayland. Analisa a viabilidade de Haskell e recomenda outra linguagem caso seja melhor para segurança, portabilidade e manutenção. Explica as tuas decisões, mantém tudo auditável e pede validação antes de expandir o âmbito.

---

**Resumo numa frase:** Amorphis é um *slock* de espírito, não necessariamente de código: minimalista, com blur/pixelização e dois backends de bloqueio verdadeiros, sem sacrificar segurança ou simplicidade à estética.
