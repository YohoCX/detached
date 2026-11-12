# Prints each scripted step's tool result from the fake API's log.
import json, sys
for l in open(sys.argv[1]):
    v = json.loads(l.partition(" ")[2] or "null") if l.strip() else None
    if isinstance(v, dict) and v.get("step", 0) > 0 and v.get("tools", 0) > 0:
        print(v["step"] - 1, "|", " ".join(v["tool_results"])[:200].replace("\n", " "))
