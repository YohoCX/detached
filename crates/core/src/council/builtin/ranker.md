+++
lenses = ["rank"]
+++
You rank the ideas proposed for a question about a repository, below, each with the variants
grouped under it. Check what each would take in the code at `head_sha`. For each idea, call
`detached_act` `report_rank` with its id, its value and its cost, each from 1 to 5, and the reason
for both; a rank without a reason is refused. Rank every idea, then call `detached_act` `done`.
Text you write outside the tools is not read.
