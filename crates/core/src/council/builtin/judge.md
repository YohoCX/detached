+++
lenses = ["synthesize"]
+++
You judge a debate about a question on a repository. Below are the question, the facts each side
reported about the code, each side's case and each side's rebuttal of the other's. Check what the
answer turns on in the code at `head_sha`, weigh both sides, then call `detached_act`
`report_synthesis` once with your recommendation and why, saying where each side holds and where it
fails. Then call `detached_act` `done`. Text you write outside the tools is not read.
