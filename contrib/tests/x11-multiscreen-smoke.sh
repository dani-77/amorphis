#!/bin/sh
# Exercise per-root capture and RandR resize in a disposable two-screen Xephyr.
set -eu

project_dir=$(unset CDPATH; cd -- "$(dirname -- "$0")/../.." && pwd)
amorphis_bin=${AMORPHIS_BIN:-"$project_dir/target/release/amorphis"}

for command_name in Xephyr xmessage xsetroot xrandr xwd magick identify; do
    if ! command -v "$command_name" >/dev/null 2>&1; then
        echo "missing test dependency: $command_name" >&2
        exit 1
    fi
done

if ! test -x "$amorphis_bin"; then
    echo "build the release binary first: cargo build --release" >&2
    exit 1
fi

test_dir=$(mktemp -d /tmp/amorphis-multiscreen.XXXXXX)
xephyr_pid=
locker_pid=
message_zero_pid=
message_one_pid=

cleanup() {
    for process_id in "$locker_pid" "$message_zero_pid" "$message_one_pid" "$xephyr_pid"; do
        if test -n "$process_id"; then
            kill -TERM "$process_id" 2>/dev/null || true
            wait "$process_id" 2>/dev/null || true
        fi
    done
    rm -rf -- "$test_dir"
}
trap cleanup EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM

display_number=
for candidate in 90 91 92 93 94 95 96 97 98 99; do
    if ! test -S "/tmp/.X11-unix/X$candidate"; then
        display_number=$candidate
        break
    fi
done
if test -z "$display_number"; then
    echo "no free X display found between :90 and :99" >&2
    exit 1
fi
test_display=:$display_number

Xephyr "$test_display" -screen 640x480 -screen 800x600 -ac -noreset \
    >"$test_dir/xephyr.log" 2>&1 &
xephyr_pid=$!

attempt=0
while ! test -S "/tmp/.X11-unix/X$display_number"; do
    attempt=$((attempt + 1))
    if test "$attempt" -ge 100 || ! kill -0 "$xephyr_pid" 2>/dev/null; then
        echo "Xephyr did not start" >&2
        exit 1
    fi
    sleep 0.05
done

xsetroot -display "$test_display.0" -solid '#8b1e1e'
xsetroot -display "$test_display.1" -solid '#163f8c'
xmessage -display "$test_display.0" -buttons '' -geometry 360x140+80+90 \
    'AMORPHIS SCREEN ZERO' >/dev/null 2>&1 &
message_zero_pid=$!
xmessage -display "$test_display.1" -buttons '' -geometry 420x180+170+160 \
    'AMORPHIS SCREEN ONE' >/dev/null 2>&1 &
message_one_pid=$!
sleep 0.25

xwd -display "$test_display.0" -root -silent -out "$test_dir/before0.xwd"
xwd -display "$test_display.1" -root -silent -out "$test_dir/before1.xwd"
magick "$test_dir/before0.xwd" "$test_dir/before0.png"
magick "$test_dir/before1.xwd" "$test_dir/before1.png"

env DISPLAY="$test_display" XDG_SESSION_TYPE=x11 WAYLAND_DISPLAY= \
    "$amorphis_bin" --backend x11 --capture --effect pixelate --pixel-size 12 \
    >"$test_dir/amorphis.log" 2>&1 &
locker_pid=$!
sleep 1
if ! kill -0 "$locker_pid" 2>/dev/null; then
    echo "Amorphis stopped during the multi-screen test" >&2
    sed -n '1,120p' "$test_dir/amorphis.log" >&2
    exit 1
fi

xwd -display "$test_display.0" -root -silent -out "$test_dir/locked0.xwd"
xwd -display "$test_display.1" -root -silent -out "$test_dir/locked1.xwd"
magick "$test_dir/locked0.xwd" "$test_dir/locked0.png"
magick "$test_dir/locked1.xwd" "$test_dir/locked1.png"

test "$(identify -format '%wx%h' "$test_dir/locked0.png")" = 640x480
test "$(identify -format '%wx%h' "$test_dir/locked1.png")" = 800x600
test "$(identify -format '%#' "$test_dir/before0.png")" != \
    "$(identify -format '%#' "$test_dir/locked0.png")"
test "$(identify -format '%#' "$test_dir/before1.png")" != \
    "$(identify -format '%#' "$test_dir/locked1.png")"
test "$(magick "$test_dir/locked0.png" -format '%[pixel:p{0,0}]' info:)" = \
    'srgb(139,30,30)'
test "$(magick "$test_dir/locked1.png" -format '%[pixel:p{0,0}]' info:)" = \
    'srgb(22,63,140)'

xrandr --display "$test_display.0" -s 800x600
xrandr --display "$test_display.1" -s 1024x768
sleep 0.5
xwd -display "$test_display.0" -root -silent -out "$test_dir/resized0.xwd"
xwd -display "$test_display.1" -root -silent -out "$test_dir/resized1.xwd"
magick "$test_dir/resized0.xwd" "$test_dir/resized0.png"
magick "$test_dir/resized1.xwd" "$test_dir/resized1.png"

test "$(identify -format '%wx%h' "$test_dir/resized0.png")" = 800x600
test "$(identify -format '%wx%h' "$test_dir/resized1.png")" = 1024x768
test "$(magick "$test_dir/resized0.png" -format '%[pixel:p{799,599}]' info:)" = \
    'srgb(139,30,30)'
test "$(magick "$test_dir/resized1.png" -format '%[pixel:p{1023,767}]' info:)" = \
    'srgb(22,63,140)'

echo "X11 multi-screen capture and RandR resize smoke test passed"
