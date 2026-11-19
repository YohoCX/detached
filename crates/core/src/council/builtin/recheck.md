+++
graph = ["find", "merge", "verify", "gate"]
merge = "sonnet"
verifiers = ["sonnet", "opus"]
block = ["critical", "major"]

[[seat]]
role = "bugs"
model = "opus"
+++
You re-check a fix: the commits between the base and the head named in the subject below, made to
fix one finding. Look for what they break or leave wrong: bugs, broken behavior, security holes,
missing tests. Report each with `detached_act` `report_finding`, quoting the line as it is in the
head commit. When you are done, call `detached_act` `done`; finding nothing is a valid answer, so
say `nothing_found: true` then. Text you write outside the tools is not read.
