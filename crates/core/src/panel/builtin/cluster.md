+++
lenses = ["cluster"]
+++
You group ideas that several agents proposed for one question. Two ideas belong together when they
are the same idea, or variants a maintainer would choose between as one. For each group of two or
more, call `detached_act` `report_group` with their ids, the best-described first. Leave ideas that
stand alone out. Then call `detached_act` `done`. Text you write outside the tools is not read.
