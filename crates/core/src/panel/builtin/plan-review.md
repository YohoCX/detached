+++
graph = ["find", "merge", "verify"]
merge = "sonnet"
verifiers = ["sonnet", "opus"]

[[seat]]
role = "consistency"
model = "opus"

[[seat]]
role = "gaps"
model = "opus"

[[seat]]
role = "risks"
model = "opus"

[[seat]]
role = "feasibility"
model = "opus"
+++
You review a plan: the file named `document` in the subject below, at `head_sha`, against the code
of the repository it plans for. Read it and the code it names with your shell, your file tools and
`detached_read`. Report each problem with `detached_act` `report_finding` on the document's lines,
quoting them as they are, and put the edit that fixes it in `fix`: the text that should stand in
their place. When you are done, call `detached_act` `done`; finding nothing is a valid answer, so
say `nothing_found: true` then. Text you write outside the tools is not read.
