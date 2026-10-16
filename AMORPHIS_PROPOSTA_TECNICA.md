# Amorphis — proposta técnica da Fase A

> **Estado:** proposta aprovada pelo autor em 2026-10-15; Fase B em curso.
>
> **Data da análise:** 2026-10-15.
>
> **Decisões confirmadas:** Rust, Wayland primeiro, licença MIT e PNG como
> primeiro formato de imagem estática.

## 1. Decisão recomendada

Implementar o Amorphis em **Rust**, num único executável sem `setuid`, com:

- um núcleo pequeno de estado, autenticação e composição de imagem;
- um backend Wayland baseado exclusivamente em `ext-session-lock-v1`;
- um backend X11 baseado em `x11rb`, com grabs verificados e as limitações de X11 claramente assumidas;
- autenticação PAM através de `libpam`, usando uma camada Rust auditada e uma conversa própria;
- renderização CPU para buffers em memória, sem OpenGL e sem toolkit gráfico geral;
- fundo inicial sempre opaco; imagem estática opcional; captura e efeitos adicionados apenas depois de os dois bloqueios reais estarem sólidos.

O primeiro backend a implementar deve ser **Wayland**. É onde o protocolo define uma transição de bloqueio real e permite validar desde cedo a máquina de estados que nunca autoriza um desbloqueio acidental. X11 vem a seguir, como backend explicitamente mais fraco.

## 2. Materiais locais examinados

### 2.1. `slock` 1.7

O tarball local é o `slock` 1.7 oficial:

- SHA-256 local: `99711e6b419b221ae9fc3e9c8f31e6b168db51154278996f3d02ce65cc198301`;
- coincide com o checksum publicado pelo projeto;
- licença MIT/X Consortium;
- implementação deliberadamente pequena: Xlib, XRandR, `crypt(3)`, janela `override_redirect`, grabs de teclado e ponteiro, e queda de privilégios depois de ler o hash.

Ideias a conservar:

- falhar se não for possível obter todos os grabs;
- manter o ciclo de estados e a interface mínimos;
- limpar o buffer da palavra-passe;
- cobrir todos os ecrãs X e reagir a mudanças RandR;
- separar inicialização privilegiada do funcionamento normal.

Código a não transportar:

- autenticação direta por `crypt(3)` e leitura de `/etc/shadow`;
- instalação `setuid` de todo o locker;
- tratamento de texto por `XLookupString` como solução comum aos dois backends;
- a suposição de que uma janela X11 equivale a um protocolo seguro de sessão;
- ausência de uma separação clara entre estado de segurança, desenho e autenticação.

### 2.2. Patch de blur/pixelização

A patch local é uma série de alterações de 2020 para `slock`, baseada em Imlib2. Aplicada sequencialmente, ainda encaixa no `slock` 1.7 local. Não foi possível compilá-la neste sistema porque os headers de Imlib2 não estão instalados; isso não impede a análise estática.

Problemas encontrados:

1. A captura e todo o processamento são feitos **antes** dos grabs. Um blur ou uma imagem grande prolongam o intervalo entre a invocação e o bloqueio efetivo.
2. Os limites dos blocos estão errados: `i < width` e `j < height` não consideram `x` e `y`. Nos blocos finais, a rotina pode consultar coordenadas fora da imagem.
3. O divisor é sempre `pixelSize * pixelSize`, mesmo em blocos parciais; as margens ficam calculadas incorretamente.
4. `pixelSize == 0` provoca ciclos infinitos se a compilação ativar `PIXELATION`.
5. As somas RGB em `int` podem transbordar para tamanhos de bloco elevados.
6. O alfa usado no bloco é apenas o do último pixel consultado.
7. A captura usa apenas o `DefaultScreen`; não modela corretamente vários X screens, geometrias diferentes nem escalas.
8. A imagem é global e libertada dentro de `lockscreen()`. Com mais de um X screen, o ponteiro continua não nulo e pode voltar a referir memória já libertada.
9. `bgmap` nasce de `malloc()` sem inicialização e pode ser testado sem valor válido se a criação da imagem falhar.
10. Não há validação de falhas nas operações de imagem/pixmap nem garantia de preenchimento opaco integral.
11. O feedback de autenticação original é substituído pelo mesmo pixmap, deixando de refletir corretamente os estados `INPUT` e `FAILED`.
12. `-Ofast` é uma alteração sem benefício demonstrado para um locker e torna as garantias de otimização menos conservadoras.

Conclusão: aproveitar apenas a intenção visual. Os algoritmos e a integração devem ser reescritos com aritmética verificada, limites explícitos e testes. A patch não contém uma licença própria explícita; por prudência, não copiar trechos substanciais sem clarificar o licenciamento. O `slock` original pode ser usado sob a sua licença MIT, preservando o aviso.

## 3. Comparação de linguagens

| Critério | Rust | C | Haskell |
|---|---|---|---|
| Wayland/session-lock | Ecossistema atual e bindings tipados; SCTK já expõe session lock | Suporte oficial e direto por `libwayland`/código gerado | Exigiria bindings/FFI próprios ou pouco consolidados |
| X11 | `x11rb` cobre protocolo e RandR com grande parte do código seguro | Xlib/XCB maduros, mas gestão manual | Binding X11 existe, mas não resolve a assimetria Wayland |
| PAM | FFI inevitável, isolável numa biblioteca/camada pequena | API nativa | FFI e marshalling adicionais |
| Memória/imagens | Buffers e aritmética podem ficar seguros por defeito | Maior risco em pixels, tamanhos e callbacks | GC dificulta apagar cópias de segredos de forma previsível |
| Binário/arranque | Adequado para um locker pequeno | Melhor resultado absoluto | RTS e empacotamento mais pesados |
| Auditoria/manutenção | Bom equilíbrio entre tipos, controlo e ecossistema | Simples apenas enquanto o programa ficar muito pequeno | Complexidade desproporcionada nas fronteiras FFI |

**Resultado:** Rust. C seria a segunda escolha aceitável. Haskell não é recomendado para este projeto: as partes difíceis são precisamente protocolos de sistema, callbacks C, buffers gráficos e limpeza de segredos, áreas em que obrigaria a mais FFI e tornaria a implementação menos pequena e menos auditável.

## 4. Bibliotecas e dependências propostas

As versões exatas serão fixadas em `Cargo.lock` e auditadas antes do primeiro release.

### Núcleo

- buffer próprio de capacidade fixa, apagado com `explicit_bzero(3)` no `Drop`,
  sem prometer apagar cópias que bibliotecas PAM possam criar internamente;
- `thiserror` é dispensável no início; erros próprios simples reduzem dependências.
- `log` também é dispensável no MVP. Mensagens controladas em `stderr`, nunca incluindo segredos, caminhos sensíveis desnecessários ou conteúdo de imagem.

### Wayland

- `smithay-client-toolkit` para outputs, seats/teclado, `wl_shm` e a abstração de session lock;
- `wayland-client` e `wayland-protocols` por baixo, com a feature de protocolos staging;
- `calloop`, já integrado no SCTK, para Wayland, canais de autenticação e sinais.

O protocolo permite gráficos arbitrários nas superfícies de lock, mas exige dimensões exatas após `configure`, superfícies para todos os outputs e `unlock_and_destroy` apenas depois do evento `locked`. Se o cliente morrer, o compositor não deve desbloquear a sessão.

### X11

- `x11rb`, apenas com as features necessárias (`randr` e `image`; SHM só depois de medição);
- `xkbcommon`/`xkbcommon-x11` para layouts e composição de teclado, aceitando uma fronteira FFI pequena;
- sem Xlib e sem toolkit GUI.

### PAM

- `libpam` do sistema;
- candidato preferido para protótipo: `pam-client2`, sem as features de CLI, por oferecer conversa PAM personalizável e API de aplicação;
- antes de o aceitar definitivamente: auditoria do caminho `PAM_PROMPT_ECHO_OFF`, alocações/respostas, `pam_end`, erros e compatibilidade com `pam_acct_mgmt`;
- não usar o helper de palavra-passe simplificado da crate `pam` 0.8: a própria documentação limita-o à autenticação básica e mantém uma questão pendente sobre fugas na conversa.

O fluxo será `pam_start` → `pam_authenticate` → `pam_acct_mgmt` → `pam_end`. Não abrir uma nova sessão PAM. A autenticação corre num worker para o event loop continuar a tratar hotplug e redesenho. Não há `setuid`; se uma política concreta exigir privilégios adicionais, será desenhado depois um helper mínimo separado, nunca toda a interface privilegiada.

### Imagem

- buffers internos RGBA8 próprios;
- `image` com `default-features = false` e apenas PNG/JPEG, se a auditoria confirmar o custo aceitável;
- limites rígidos de dimensões, número de píxeis e memória antes de descodificar ou alocar;
- pixelização própria, com blocos parciais corretos e aritmética verificada;
- blur CPU separável ou `fast_blur`, medido e testado antes da escolha final;
- saída Wayland por `wl_shm`; saída X11 pelo módulo `image` do `x11rb`, que converte para o formato nativo do servidor.

Não usar Imlib2 no desenho inicial: não é necessário para a captura bruta, aumenta a superfície nativa e não resolve a portabilidade Wayland.

## 5. Arquitetura e fronteiras de segurança

```text
                  CLI/configuração validada
                            |
                    máquina de estados
              +-------------+-------------+
              |             |             |
        backend ativo   autenticação    compositor RGBA
          /       \        PAM worker      /       \
       Wayland    X11                    efeitos   imagem
          |         |                    seguros   estática
   session-lock   grabs
```

A interface comum deve ser pequena: eventos de input/output, pedido de redesenho e a única operação terminal `unlock_after_authentication`. As implementações Wayland e X11 não devem ser forçadas a partilhar sequências que têm garantias diferentes.

Estados mínimos:

```text
Starting -> LockRequested -> Locked -> Authenticating -> Locked
                              |              |
                              |              +-> Authenticated -> Unlocking -> Exit
                              +-> fatal: permanecer coberto/falhar fechado quando possível
```

Regras invariantes:

- nunca chamar a operação de desbloqueio antes de `Locked` **e** de uma resposta PAM bem-sucedida;
- erros, cancelamentos e panics nunca contam como autenticação;
- o primeiro frame de cada output é opaco;
- um output novo recebe imediatamente uma superfície opaca;
- nenhuma palavra-passe entra em logs, argumentos, ambiente ou ficheiros;
- todos os tamanhos de imagem usam multiplicações verificadas e limites configurados internamente;
- se a imagem, captura ou efeito falhar, o backend conserva o fundo opaco.

## 6. Fluxos por backend

### 6.1. Wayland

1. Ligar ao compositor e verificar `ext_session_lock_manager_v1`.
2. Se não existir, terminar com erro claro. Não usar `layer-shell` nem cair automaticamente para XWayland.
3. Pedir o lock e criar uma `ext_session_lock_surface_v1` por output presente.
4. Após cada `configure`, confirmar o serial e anexar imediatamente um buffer `wl_shm` opaco nas dimensões exigidas.
5. Só considerar o estado seguro após receber `locked`.
6. Tratar outputs novos/removidos e novas configurações enquanto bloqueado.
7. Processar input e PAM sem bloquear o event loop.
8. Após sucesso PAM, enviar `unlock_and_destroy`, fazer `wl_display.sync`/roundtrip e só depois sair.

Uma imagem estática pode substituir o buffer sólido depois de descodificada. Se falhar, a cor permanece. Isto não depende de captura nem de permissões do compositor.

### 6.2. X11

1. Criar janelas opacas `override_redirect` para todos os X screens, ainda não assumindo que estão seguros.
2. Obter e verificar os grabs de teclado e ponteiro; uma falha em qualquer screen aborta todo o lock.
3. Mapear e elevar as janelas, sincronizar e só então anunciar estado bloqueado.
4. Reagir a RandR, recriando/redimensionando cobertura antes de atualizar a imagem decorativa.
5. Manter tentativas de elevação perante alterações de subestrutura.
6. Apenas terminar após PAM bem-sucedido.

A captura implementada usa uma secção crítica curta com `GrabServer`: captura o root, obtém os grabs, mapeia as janelas e sincroniza antes de libertar o servidor. Mesmo assim, X11 não recebe garantias equivalentes a Wayland; VT switching, reset/encerramento do servidor e clientes com acesso ao mesmo servidor continuam limitações documentadas.

## 7. Política de fundos e captura

Ordem de fallback:

1. captura anterior ao bloqueio + efeito pedido;
2. imagem estática configurada;
3. cor sólida configurada/compilada.

A presença de imagem estática não atrasa o bloqueio: o backend apresenta primeiro a cor e troca de buffer quando a imagem estiver pronta.

Modos iniciais para a imagem:

- `cover`: preenche e corta mantendo proporção; recomendado;
- `contain`: mostra tudo e preenche margens com a cor;
- `center`: sem escala, com corte/margens opacas.

A captura Wayland é uma capacidade opcional, nunca condição para bloquear:

- preferir `ext-image-copy-capture-v1` + `ext-output-image-capture-source-v1` quando ambos forem anunciados e a política do compositor autorizar;
- captura por portal exige normalmente seleção/consentimento e PipeWire, aumentando dependências e latência; não faz parte do MVP automático;
- fluxo implementado: capturar rapidamente para memória → pedir session lock → mostrar buffers sólidos → processar blur/pixelização já bloqueado → substituir buffers;
- impor timeout curto; qualquer ausência, recusa ou falha segue diretamente para imagem estática/cor.

Não criar um daemon residente apenas para manter screenshots recentes: seria complexidade e retenção de conteúdo sensível desnecessárias.

## 8. Configuração inicial proposta

CLI, sem ficheiro de configuração na primeira versão:

```text
amorphis [--backend auto|wayland|x11]
         [--background solid|image|capture]
         [--image PATH]
         [--fit cover|contain|center]
         [--effect none|blur|pixelate|blur-pixelate]
         [--blur-radius N]
         [--pixel-size N]
         [--color '#RRGGBB']
```

Regras:

- `auto` prefere Wayland sempre que `WAYLAND_DISPLAY` existir; não escolhe XWayland por conveniência;
- `--background image` exige `--image`; falha de leitura depois da validação visual conserva a cor e regista apenas um erro não sensível;
- limites numéricos estreitos e documentados; zero não é válido para raio/tamanho quando o efeito correspondente está ativo;
- sem temas, plugins, comandos pós-lock ou configuração executável.

## 9. Plano incremental

### Etapa 0 — fundações

- criar projeto Cargo, licença, política de dependências e CI;
- implementar máquina de estados e compositor RGBA sem acesso ao display;
- testes unitários de transições, escalamento, margens de pixelização, overflow e falhas de alocação.

### Etapa 1 — PAM isolado

- configurar o serviço PAM `amorphis` de exemplo;
- implementar conversa e worker;
- testar palavra-passe correta/incorreta, conta bloqueada/expirada, módulo ausente, mensagens múltiplas e cancelamento;
- confirmar limpeza dos buffers controlados pelo Amorphis.

### Etapa 2 — Wayland sólido

- implementar `ext-session-lock-v1`, outputs, `wl_shm`, teclado e estado de foco;
- integrar PAM;
- testar apenas num compositor aninhado durante o desenvolvimento até existir recuperação segura;
- recusar compositores sem o protocolo.

### Etapa 3 — X11 sólido

- implementar janelas, grabs, input e RandR com `x11rb`;
- integrar a mesma máquina de autenticação;
- documentar diferenças e vias de bypass inerentes a X11.

### Etapa 4 — imagem estática

- descodificação limitada, `cover`/`contain`/`center`, vários outputs e escalas;
- garantir que qualquer falha mantém a cor sólida.

### Etapa 5 — captura e efeitos

- X11 primeiro, por a captura do root estar disponível no mesmo protocolo;
- Wayland apenas por capacidades anunciadas, começando pelos protocolos `ext-image-*`;
- blur/pixelização comuns e testados com imagens pequenas, margens parciais e limites extremos.

### Etapa 6 — endurecimento e empacotamento

- fuzzing do parser/configuração e processamento de imagem;
- `cargo audit`/`cargo deny`, revisão de `unsafe`, sanitizers onde aplicável;
- pacotes e serviço PAM para distribuições piloto;
- testes reais de hotplug, suspensão/retoma, perda de compositor e pressão de memória.

## 10. Critérios de aceitação

### Gerais

- uma autenticação falhada, erro PAM, panic no worker ou falha visual nunca chama unlock;
- palavras-passe não aparecem em logs, core dumps controláveis, argv, ambiente ou ficheiros temporários;
- a imagem original nunca é escrita em disco pelo Amorphis;
- falha de imagem/efeito produz fundo opaco e mantém input funcional;
- latência e memória têm limites medidos e publicados.

### Wayland

- nenhum sucesso é reportado antes do evento `locked`;
- todos os outputs ativos têm frame opaco antes de o compositor confirmar o lock;
- hotplug e reconfiguração mantêm cobertura;
- ausência/recusa do protocolo termina sem alegar que a sessão ficou bloqueada;
- queda do processo não pede desbloqueio; recuperação fica a cargo da política segura do compositor;
- unlock só ocorre depois de PAM e é sincronizado antes da saída.

### X11

- todos os grabs devolvem `GrabSuccess` antes de o estado passar a bloqueado;
- falha parcial desfaz o processo e retorna erro;
- todas as geometrias do root permanecem cobertas após eventos RandR;
- teclas/rato não chegam a clientes normais enquanto os grabs persistem;
- documentação afirma explicitamente que matar o locker, trocar de VT ou terminar o servidor pode contornar o modelo, conforme a configuração do sistema.

## 11. Riscos e decisões ainda abertas

1. **Licença do Amorphis:** MIT, confirmada pelo autor e aplicada ao repositório.
2. **Crate PAM:** `pam-client2` foi revista no caminho usado e fixada no
   lockfile. Em 2026-10-16, `cargo audit` analisou as 75 crates contra 1 329
   avisos RustSec sem encontrar vulnerabilidades conhecidas; a auditoria deve
   ser repetida regularmente. A crate e o próprio PAM criam cópias do segredo
   que o Amorphis não consegue bloquear ou apagar.
3. **Formatos de imagem:** PNG foi confirmado e é o único formato ativado;
   JPEG só será considerado com justificação adicional.
4. **Captura Wayland:** suporte real varia por compositor. O produto deve anunciar capacidades detetadas, não uma lista rígida de desktops.
5. **Comportamento de sinais no X11:** não há equivalente ao fail-closed do compositor. É limitação arquitetural, não algo que a UI possa corrigir.
6. **Core dumps/swap:** core dumps e dumpabilidade Linux são desativados antes
   do lock. O buffer controlado usa `mlock` de melhor esforço e avisa se o
   limite do sistema impedir a proteção; as cópias internas de PAM continuam
   fora deste controlo.

## 12. Fontes técnicas consultadas

- slock: <https://tools.suckless.org/slock/>
- checksums oficiais: <https://dl.suckless.org/tools/sha256sums.txt>
- patch blur/pixelização: <https://tools.suckless.org/slock/patches/blur-pixelated-screen/>
- `ext-session-lock-v1`: <https://wayland.app/protocols/ext-session-lock-v1>
- `ext-image-copy-capture-v1`: <https://wayland.app/protocols/ext-image-copy-capture-v1>
- `ext-image-capture-source-v1`: <https://wayland.app/protocols/ext-image-capture-source-v1>
- SCTK session lock: <https://docs.rs/smithay-client-toolkit/latest/smithay_client_toolkit/session_lock/>
- exemplo de session lock do SCTK: <https://docs.rs/crate/smithay-client-toolkit/latest/source/examples/session_lock.rs>
- `x11rb`: <https://docs.rs/x11rb/latest/x11rb/>
- Linux-PAM API: <https://github.com/linux-pam/linux-pam/blob/master/doc/adg/Linux-PAM_ADG.xml>
- portal ScreenCast: <https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.ScreenCast.html>
- `zeroize`: <https://docs.rs/zeroize/latest/zeroize/>
- limites da crate `image`: <https://docs.rs/image/latest/image/struct.Limits.html>

## 13. Decisão aprovada

Avançar com Rust, Wayland primeiro, PAM sem privilégios globais e um MVP que
bloqueia com cor sólida. A imagem estática vem antes da captura e dos efeitos.
A patch é referência visual, não código-base.

Esta sequência mantém o projeto utilizável e auditável em cada marco e impede que a parte estética esconda falhas no mecanismo de bloqueio.

## 14. Progresso da implementação

Concluído no primeiro incremento da Fase B:

- estrutura Cargo, licença MIT e lockfile de dependências;
- máquina de estados com desbloqueio exclusivamente após sucesso PAM;
- buffer de palavra-passe limitado e apagado explicitamente;
- worker PAM não bloqueante e política PAM de exemplo;
- backend Wayland por `ext-session-lock-v1`, incluindo outputs dinâmicos,
  teclado e buffers `wl_shm` opacos;
- PNG estático carregado num worker para Wayland e X11, com limites e modos
  `cover`, `contain` e `center`, sempre com fallback sólido;
- efeitos de blur e pixelização ligados à CLI comum dos dois backends;
- backend X11 com fundos sólidos/PNG, grabs verificados, XKB, múltiplos X
  screens e RandR;
- captura do root X11 em memória, com transição curta protegida por
  `GrabServer`, fallback incorporado e efeitos processados depois da cobertura;
- captura Wayland por output com `ext-image-copy-capture-v1`, timeout curto,
  fallback incorporado e efeitos processados depois do pedido de session lock;
- indicador de autenticação comum aos dois backends, sem revelar comprimento:
  input verde-azulado, três pontos fixos animados durante PAM e erro vermelho;
- desativação de core dumps antes de aceitar input sensível, limpeza explícita
  e `mlock` de melhor esforço no buffer controlado da palavra-passe;
- auditoria RustSec inicial das 75 crates do lockfile sem vulnerabilidades
  conhecidas;
- testes unitários, `clippy -D warnings` e build otimizada.

Ainda por validar/implementar: a matriz alargada de compositores/servidores,
multi-monitor físico, ciclos adicionais de suspensão/retoma e PAM por
distribuição; validação real da captura Wayland fora de Wasp/wlroots e
endurecimento final de empacotamento. A captura X11 com
blur/pixelização já foi validada numa sessão Xorg real. Duas capturas
independentes e o redimensionamento RandR foram também validados num Xephyr
descartável com dois X screens de resoluções diferentes. No Wayland, um Wasp
nested com dois outputs confirmou cobertura simultânea e capturas pixelizadas
distintas por `wl_output`. Um ciclo real de suspensão/retoma sob Wasp manteve a
cobertura com captura e efeitos, recusou duas tentativas PAM falhadas e só
desbloqueou após autenticação correta.
O indicador foi validado automaticamente sob Xephyr com input injetado,
expiração do estado de input e uma tentativa PAM deliberadamente rejeitada.
Input, animação durante PAM, erro temporizado e desbloqueio correto foram
também confirmados visualmente numa sessão Wayland real.
