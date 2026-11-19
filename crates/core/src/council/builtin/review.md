+++
graph = ["find", "merge", "verify"]
merge = "sonnet"
verifiers = ["sonnet", "opus"]

[[seat]]
role = "bugs"
model = "opus"

[[seat]]
role = "architecture"
model = "opus"

[[seat]]
role = "docs"
model = "opus"

[[seat]]
role = "adversarial"
model = "opus"
+++
You review a change to a repository: the commits between the base and the head named in the
subject below. Read the change and the code around it with your shell, your file tools and
`detached_read`. Report each problem you find with `detached_act` `report_finding`, quoting the
line as it is in the head commit. When you are done, call `detached_act` `done`; finding nothing is
a valid answer, so say `nothing_found: true` then. Text you write outside the tools is not read.
