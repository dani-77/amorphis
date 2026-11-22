# Configurar Amorphis e amorphis-idle

Os dois programas procuram ficheiros em
`${XDG_CONFIG_HOME:-$HOME/.config}/amorphis/`. Com a configuração habitual,
isso corresponde a `~/.config/amorphis/`:

| Ficheiro | Utilizado por | Obrigatório? |
| --- | --- | --- |
| `x11.toml` | `amorphis` numa sessão X11 | Não |
| `wayland.toml` | `amorphis` numa sessão Wayland | Não |
| `idle.toml` | `amorphis-idle` nas duas sessões | Sim, para iniciar o daemon |

Os exemplos são instalados em `PREFIX/share/amorphis/examples/`, normalmente
`/usr/local/share/amorphis/examples/` ou `/usr/share/amorphis/examples/`. Para
começar com a instalação padrão:

```sh
config_dir="${XDG_CONFIG_HOME:-$HOME/.config}/amorphis"
mkdir -p "$config_dir"
cp /usr/local/share/amorphis/examples/*.toml "$config_dir/"
```

Se a instalação usou `PREFIX=/usr`, troca `/usr/local/share` por `/usr/share`.
Os perfis de exemplo têm apenas comentários e mantêm o fundo incorporado até
serem editados. O `idle.toml` de exemplo contém uma regra ativa de cinco minutos.
O serviço PAM `amorphis` tem de estar instalado separadamente; consulta
[a política PAM](contrib/pam/README.md).
Em Linux, não executes o locker num sandbox com `NoNewPrivs`: políticas PAM
como `pam_unix` podem precisar de um auxiliar privilegiado. O Amorphis deteta
essa condição e recusa o bloqueio antes de cobrir o ecrã.

## Perfis do locker

Ao chamar apenas `amorphis`, o programa deteta a sessão e lê o perfil
correspondente **a cada invocação**. Se o ficheiro não existir, usa o fundo
incorporado. Um perfil com TOML inválido, opção desconhecida ou combinação
inválida é reportado com o caminho do ficheiro e impede a tentativa de bloqueio.
Uma imagem que falhe durante o carregamento usa os fundos de recurso sem
cancelar um bloqueio já iniciado.

As opções do perfil ficam na raiz do ficheiro, sem secções TOML:

| Chave | Valor | Comportamento |
| --- | --- | --- |
| `image` | Caminho PNG não vazio | Usa uma imagem; recomenda-se um caminho absoluto. |
| `capture` | `true` ou `false` | Captura o ambiente de trabalho quando `true`. |
| `solid` | `true` ou `false` | Usa só a cor sólida quando `true`. |
| `fit` | `"cover"`, `"contain"` ou `"center"` | Ajusta a imagem ao output; padrão `"cover"`. |
| `effect` | `"none"`, `"blur"`, `"pixelate"` ou `"blur-pixelate"` | Aplica o efeito; padrão `"none"`. |
| `blur_radius` | Inteiro de 1 a 128 | Raio do blur; padrão 16 quando aplicável. |
| `pixel_size` | Inteiro positivo | Tamanho dos blocos; padrão 16 quando aplicável. |

Escolhe apenas uma origem de fundo: `image`, `capture = true` ou
`solid = true`. `solid = true` não pode ser combinado com `fit`, efeitos ou
parâmetros de efeitos. `blur_radius` exige `effect = "blur"` ou
`"blur-pixelate"`; `pixel_size` exige `"pixelate"` ou `"blur-pixelate"`.
Os nomes em TOML usam `_`, como `blur_radius`, enquanto a CLI usa `-`, como
`--blur-radius`.

Exemplo de `x11.toml` com captura e ambos os efeitos:

```toml
capture = true
effect = "blur-pixelate"
blur_radius = 16
pixel_size = 16
```

Exemplo de `wayland.toml` com uma imagem:

```toml
image = "/home/utilizador/Imagens/fundo.png"
fit = "cover"
effect = "pixelate"
pixel_size = 12
```

Substitui o caminho pelo teu ficheiro. A captura Wayland depende dos protocolos
anunciados pelo compositor; se não estiver disponível, o locker mantém um
fundo opaco de recurso. Para um fundo simples, usa apenas `solid = true`.

As chamadas explícitas, como `amorphis --backend x11 --solid`, continuam a
funcionar e **não leem** `x11.toml` nem `wayland.toml`.
Numa sessão Wayland com XWayland, a presença de `DISPLAY` não seleciona X11.

## Regras de inatividade

`amorphis-idle` lê `idle.toml` **uma vez ao iniciar**. Depois de editar o
ficheiro, reinicia o daemon. Cada bloco `[[rule]]` tem um `timeout` inteiro
positivo em segundos e pelo menos um comando aplicável à sessão. Por exemplo:

```toml
[[rule]]
timeout = 300
command = "amorphis"
```

Se o comando for `amorphis`, uma alteração em `x11.toml` ou `wayland.toml`
entra em vigor no próximo bloqueio, sem reiniciar `amorphis-idle`. O timeout
pertence apenas a `idle.toml`, não aos perfis do locker.

`300` são cinco minutos; `60` é um minuto e `900` são quinze minutos. Cada
regra dispara uma vez por período de inatividade e é rearmada quando há
atividade. Podem existir várias regras com limiares diferentes. Evita
configurar vários lockers para o mesmo período.

Para escolher o locker conforme a sessão, mantendo um único timeout:

```toml
[[rule]]
timeout = 300
command_x11 = "slock"
command_wayland = "amorphis"
```

Se houver também `command`, `command_x11` tem precedência em X11 e
`command_wayland` em Wayland. A mesma sessão usada para detetar a inatividade
é usada para escolher o comando. Uma regra sem comando aplicável faz o daemon
terminar com um erro que identifica a regra.

Opcionalmente, `resume_command` corre quando a atividade regressa **depois**
de a regra ter disparado:

```toml
[[rule]]
timeout = 900
command = "amorphis"
resume_command = "notify-send 'Atividade retomada'"
```

O regresso da atividade pode ocorrer ao começar a escrever a palavra-passe.
Assim, `resume_command` **não é um comando pós-desbloqueio** e pode executar
enquanto o locker continua aberto. Os comandos são enviados a `sh -c`, herdam
o ambiente da sessão e não bloqueiam a deteção de inatividade. Falhas de
lançamento ou saídas sem sucesso são escritas em `stderr`. Usa caminhos
absolutos se o `PATH` do arranque da sessão for reduzido.

Inicia `amorphis-idle` uma vez no arranque da sessão, por exemplo com
`amorphis-idle &` no script de arranque do ambiente. Não necessita de systemd.
Em X11 usa MIT-SCREEN-SAVER; em Wayland precisa de `ext-idle-notify-v1` e
respeita as notificações normais do compositor. O daemon ainda não gere
suspensão, D-Bus nem inibidores próprios. Quando faltar um protocolo
necessário, termina com uma mensagem de erro.
