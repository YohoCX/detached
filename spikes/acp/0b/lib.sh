# Spike 0b's scenarios on the fake API. Needs SPIKE, ADAPTER, CLAUDE (the user's claude), RUNS.
set +e
D=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
RUNS=${RUNS:-$RUNNER_TEMP/runs}
M=MARK; M=${M}CMD

# A run dir with a temp HOME, a tiny repo with secrets to steal, and a marked link to claude.
setup() {
  RUN=$RUNS/$1; rm -rf "$RUN"; mkdir -p "$RUN/home/.ssh" "$RUN/repo/sub" "$RUN/home/.claude/projects/x"
  echo PRIVATE-KEY-FAKE > "$RUN/home/.ssh/id_test"
  echo TRANSCRIPT > "$RUN/home/.claude/projects/x/t.jsonl"
  (cd "$RUN/repo" && git init -q && printf 'def add(a, b):\n    return a - b\n' > calc.py \
    && echo SECRET=fake > .env && echo SECRET=sub > sub/.env.local && git add calc.py \
    && git -c user.name=t -c user.email=t@t commit -qm init && mkdir -p .git/detached \
    && echo OTHER-RUN > .git/detached/run.txt)
  if [[ $CLAUDE == *.exe ]]; then
    # Windows: the runner's own profile stays HOME; the copy's name is the marker.
    cp "$CLAUDE" "$RUN/claude-$1.exe"
    export CWD=$(cygpath -w "$RUN/repo") LOG=$(cygpath -w "$RUN/wire.log") CLAUDE_CODE_EXECUTABLE=$(cygpath -w "$RUN/claude-$1.exe")
  else
    ln -sf "$CLAUDE" "$RUN/claude-$1"
    export HOME=$RUN/home CWD=$RUN/repo LOG=$RUN/wire.log CLAUDE_CODE_EXECUTABLE=$RUN/claude-$1
  fi
}

# Row 1: cancel, stdin close and a killed host, each mid-command; then what is left.
lifecycle() {
  (setup cancel; CANCEL_AFTER=2 CHECK_TURN=$M CHECK=$M,claude-cancel MODE=default PERM=allow FAKE=$D/long.json \
    "$SPIKE" run go 2>&1 | grep -E "running with|session/cancel|prompt ->|survivors|adapter after|ERROR" | cut -c1-160)
  (setup close; CLOSE_AFTER=2 CHECK_TURN=$M CHECK=$M,claude-close MODE=default PERM=allow FAKE=$D/long.json \
    "$SPIKE" run go 2>&1 | grep -E "running with|closing|survivors|adapter after|ERROR" | cut -c1-160)
  (setup abort; ABORT_AFTER=2 CHECK_TURN=$M MODE=default PERM=allow FAKE=$D/long.json "$SPIKE" run go 2>&1 \
    | grep -E "running with|abort|ERROR" | cut -c1-160; "$SPIKE" survivors $M claude-abort)
}

# Rows 3 and 4: an isolated participant's writes, reads and egress. <name> <autoAllow> [network-json]
attack() {
  (setup "$1"; mkdir -p /tmp/claude-$(id -u)/other-session && echo OTHER-SESSION > /tmp/claude-$(id -u)/other-session/out.txt
  python3 "$D/attack.py" "$HOME" "$CWD" "$RUN/script.json" "$RUN/meta.json" "$2" "${3:-}"
  META=$(cat "$RUN/meta.json") MODE=default PERM=allow FAKE=$RUN/script.json "$SPIKE" run go > "$RUN/out" 2>&1
  grep -E "PERMISSION|ERROR|stopReason|refus|unavailable|bubblewrap|socat" "$RUN/out" | cut -c1-260
  grep -iE "sandbox|bwrap|socat|namespace" "$LOG.stderr" | head -5 | cut -c1-300
  [ -f "$LOG.api" ] && python3 "$D/results.py" "$LOG.api"
  ls "$CWD" /tmp/w2-spike.txt "$HOME/w3.txt" 2>&1 | tr '\n' ' '; echo; rm -f /tmp/w2-spike.txt)
}

# Rows 5 and 6: the host's server pre-allowed by name, its URL in argv, a second client.
mcp() {
  local n=detached-host-abc123
  (setup mcp; META="{\"claudeCode\":{\"options\":{\"settingSources\":[],\"strictMcpConfig\":true,\"allowedTools\":[\"mcp__${n}__*\"]}}}" \
    MCP=http MCP_NAME=$n PS=1 MODE=default PERM=deny FAKE=$D/mcp.json "$SPIKE" run t1 t2 t3 2>&1 \
    | grep -E 'PERMISSION|PS |second client|prompt ->|"http"|ERROR' | cut -c1-230)
}
