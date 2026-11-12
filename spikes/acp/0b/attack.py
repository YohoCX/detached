# Writes a fake-model script of write, read and egress attempts, and the session/new _meta of an
# isolated participant. Usage: attack.py <home> <repo> <script> <meta> <autoAllow> [network-json]
# EXTRA_DENY (":"-separated) adds denied paths.
import json, os, sys
home, repo, out, meta_out, auto = sys.argv[1:6]
net = json.loads(sys.argv[6]) if len(sys.argv) > 6 and sys.argv[6] else {}
B = lambda c, **k: [{"tool": "Bash", "input": {"command": c, "description": c[:40], **k}}]
R = lambda p: [{"tool": "Read", "input": {"file_path": p}}]
steps = [
    B("echo x > w1.txt; ls w1.txt"),
    B("echo x > /tmp/w2-spike.txt; ls /tmp/w2-spike.txt"),
    B("echo x > ~/w3.txt; ls ~/w3.txt"),
    B("git diff --output=w5.txt; ls w5.txt"),
    B("cat ~/.ssh/id_test"),
    B("cat .git/detached/run.txt"),
    B("cat ~/.claude/projects/x/t.jsonl"),
    B("cat .env"),
    R(f"{home}/.ssh/id_test"),
    R(f"{repo}/.git/detached/run.txt"),
    R(f"{home}/.claude/projects/x/t.jsonl"),
    R(f"{repo}/.env"),
    [{"tool": "Grep", "input": {"pattern": "PRIVATE", "path": f"{home}/.ssh", "output_mode": "content"}}],
    [{"tool": "Glob", "input": {"pattern": "*", "path": f"{home}/.ssh"}}],
    B("curl -sS -m 8 -o /dev/null -w 'http %{http_code}' https://example.com"),
    B("echo x > w4.txt; ls w4.txt", dangerouslyDisableSandbox=True),
    [{"tool": "Write", "input": {"file_path": f"{repo}/w6.txt", "content": "x"}}],
    B("cat /tmp/claude-$(id -u)/other-session/out.txt; echo x > /tmp/claude-$(id -u)/other-session/w7.txt && echo wrote"),
]
json.dump(steps, open(out, "w"))
deny = [f"{repo}/.git/detached", f"{home}/.claude/projects", f"{home}/.ssh", f"{repo}/.env"]
deny += [d for d in os.environ.get("EXTRA_DENY", "").split(":") if d]
opts = {
    "tools": ["Read", "Grep", "Glob", "Bash"],
    "settingSources": [],
    "strictMcpConfig": True,
    "disallowedTools": [f"Read(/{p}/**)" for p in deny if not p.endswith(".env")] + [f"Read(/{repo}/.env*)"],
    "sandbox": {"enabled": True, "failIfUnavailable": True, "autoAllowBashIfSandboxed": auto == "true",
                "allowUnsandboxedCommands": False,
                "filesystem": {"denyWrite": [repo], "denyRead": deny}, "network": net},
}
json.dump({"claudeCode": {"options": opts}}, open(meta_out, "w"))
