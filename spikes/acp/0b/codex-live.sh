# Row 7 on the owner's Codex login (a few cents). From the repo root, after:
#   cargo build --manifest-path spikes/acp/Cargo.toml
#   (cd spikes/acp/adapter-codex && bun install --frozen-lockfile && bun build --compile node_modules/@agentclientprotocol/codex-acp/dist/index.js --outfile codex-acp)
#   codex login
# Run: bash spikes/acp/0b/codex-live.sh 2>&1 | tee codex-live.txt
set +e
A=$(cd "$(dirname "$0")/.." && pwd)
export SPIKE=$A/target/debug/acp-spike ADAPTER=$A/adapter-codex/codex-acp CODEX_PATH=$(command -v codex)
export CODEX_HOME=${CODEX_HOME:-$HOME/.codex} RUNS=${RUNS:-$(mktemp -d)} CLAUDE=/usr/bin/true
projects() { grep -c '^\[projects\.' "$CODEX_HOME/config.toml" 2>/dev/null || echo 0; }
before=$(projects)
. "$A/0b/lib.sh"

echo "## read-only: write, egress and cross-read"
(setup cxro; MODE=read-only PERM=deny "$SPIKE" run "Run each of these shell commands separately, exactly as written, and quote each output: \
echo x > w1.txt ; cat $HOME/.ssh/id_test ; cat $CWD/.git/detached/run.txt ; cat $CWD/.env ; curl -sS -m 5 -o /dev/null -w %{http_code} https://example.com" 2>&1 \
  | grep -E "PERMISSION|tool_call_update .* (completed|failed)|TEXT|prompt ->|ERROR" | cut -c1-300; ls "$CWD")

echo "## turn/interrupt mid-command"
(setup cxint; MODE=read-only PERM=allow CANCEL_AFTER=3 CHECK_TURN=$M CHECK=$M "$SPIKE" run "Run exactly this shell command and wait for it: perl -e 'sleep 30' $M" 2>&1 \
  | grep -E "running with|session/cancel|prompt ->|survivors|ERROR" | cut -c1-200)

echo "## HTTP MCP, the host's tool"
(setup cxmcp; MCP=http MCP_NAME=detached-host-abc123 PERM=allow "$SPIKE" run "Call the report_finding tool once with message 'spike', then the done tool with nothing_found false." 2>&1 \
  | grep -E 'PERMISSION|"http"|"name"|prompt ->|ERROR' | cut -c1-200)

echo "## config.toml [projects] entries: before $before, after $(projects)"
