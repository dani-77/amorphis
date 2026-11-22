# Amorphis e daemon de inatividade

## Objetivo

Distribuir dois executáveis no mesmo repositório. `amorphis` bloqueia a sessão. `amorphis-idle` é o nome provisório do daemon que deteta inatividade e executa comandos. O daemon usa o mesmo binário e o mesmo `idle.toml` em X11 e Wayland. Pode chamar o Amorphis ou qualquer outro locker instalado.

Na `main`, `src/main.rs` ainda exige `--backend x11` ou `--backend wayland` e passa as opções de imagem e efeitos a `src/cli.rs`. Começar por confirmar esse ponto antes de alterar a CLI.

## Amorphis sem argumentos

- Fazer com que `amorphis` detete a sessão e leia o perfil correspondente de `${XDG_CONFIG_HOME:-$HOME/.config}/amorphis/`.
- Usar `x11.toml` para X11 e `wayland.toml` para Wayland. Os perfis podem ter imagens, captura e efeitos diferentes. Ler o ficheiro a cada invocação para que uma edição se reflita no próximo bloqueio.
- Preservar as invocações atuais com `--backend` e as opções da CLI. Reutilizar a validação que já existe para as opções equivalentes em TOML.
- Se não houver perfil, usar os defaults atuais do backend. Definir e testar como reportar um perfil inválido, de modo a evitar uma falha de bloqueio que passe despercebida.
- Partilhar a deteção de sessão entre os dois binários. Tratar XWayland como parte da sessão Wayland: a presença de `DISPLAY` não deve levar a escolher X11. Quando o protocolo Wayland necessário estiver indisponível, devolver um erro claro.

## Daemon de inatividade

- Acrescentar um segundo executável Cargo no repositório, com os backends de deteção de inatividade separados da lógica de regras e de execução de comandos.
- Em X11, usar `x11rb` com a feature `screensaver` e consultar o contador da extensão MIT-SCREEN-SAVER. Converter os limiares e o retorno da atividade em eventos para o núcleo.
- Em Wayland, usar as bibliotecas que já estão no projeto e `ext-idle-notify-v1`. Registar cada timeout para o seat e consumir `Idled` e `Resumed` sem polling.
- Executar cada regra uma vez por período de inatividade e rearmá-la quando houver atividade. Se a regra tiver um comando de retorno, executá-lo quando a atividade regressar depois de a regra ter disparado.
- Lançar os comandos sem bloquear a deteção e recolher o estado dos processos filhos. Registar falhas de execução. O daemon corre na sessão do utilizador, sem dependência de systemd.
- Deixar suspensão, integração D-Bus e inibidores próprios para uma fase posterior. Em Wayland, usar as notificações normais do compositor para respeitar os seus inibidores.

## Configuração do daemon

Ler `idle.toml` da mesma pasta dos perfis do Amorphis. O timeout é configurável por regra, em segundos. Cinco minutos é o valor do exemplo, que pode ser alterado para `60`, `900` ou outro valor válido.

```toml
[[rule]]
timeout = 300
command = "amorphis"
```

A mesma regra também pode escolher um locker diferente para cada sessão, com o timeout definido uma vez só.

```toml
[[rule]]
timeout = 300
command_x11 = "slock"
command_wayland = "kantele-locker"
```

O comando específico da sessão tem precedência sobre `command`, quando os dois estiverem presentes. Reportar uma regra sem comando aplicável à sessão ativa. Aplicar a mesma seleção do backend que está a ser usado para detetar idle.

## Instalação

Adicionar um `Makefile` com suporte para `PREFIX` e `DESTDIR`. Com `PREFIX=/usr`, instalar os dois executáveis em `/usr/bin/` e os exemplos `x11.toml`, `wayland.toml` e `idle.toml` em `/usr/share/amorphis/examples/`. O template XBPS pode usar o mesmo alvo de instalação. Preservar o procedimento existente para instalar a política PAM da distribuição. Os exemplos instalados são para copiar e editar na pasta de configuração do utilizador.

## Verificação da primeira versão

1. Confirmar que os comandos existentes com `--backend` continuam a funcionar.
2. No PWM em Xorg e no Wasp em Wayland, chamar apenas `amorphis` e confirmar que carrega o perfil certo. Editar o perfil e confirmar que é relido no próximo bloqueio.
3. Usar o mesmo `idle.toml` nas duas sessões e passar por dois ciclos seguidos de inatividade, bloqueio, desbloqueio e retorno. Confirmar um disparo por ciclo.
4. Testar a seleção de `slock` em X11 e `kantele-locker` em Wayland. Confirmar a escolha do backend Wayland também quando existir `DISPLAY`.
5. Fazer uma instalação de teste com `PREFIX=/usr` e `DESTDIR` temporário e verificar os dois binários e os três exemplos.
