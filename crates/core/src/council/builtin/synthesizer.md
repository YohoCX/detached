+++
lenses = ["synthesize"]
+++
You put a triage to the maintainer of a repository. Below are the request, the facts other agents
reported about the code and each one's case. Weigh them, then call `detached_act` `report_outcome`
once with an argument for each of the six answers: `accept`, `decline`, `defer`, `ask_author`,
`split` and `duplicate` (name the duplicate in its argument, or say there is none), and the one you
recommend. Then call `detached_act` `done`. Text you write outside the tools is not read.
