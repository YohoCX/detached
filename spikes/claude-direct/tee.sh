#!/bin/sh
# Stands in for `claude` under the Agent SDK: records the argv, the env's names and both pipes.
printf '%s\n' "$@" > "$CAPTURE/argv"
env | cut -d= -f1 | sort > "$CAPTURE/env"
tee "$CAPTURE/stdin" | "$CLAUDE" "$@" | tee "$CAPTURE/stdout"
