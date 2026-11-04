//! The MR, pipeline, job and GraphQL routes. Each returns `None` for a route it does not serve.

use crate::http::{Reply, Req};
use crate::{Kind, State};

/// Routes outside the APIs (`/__append`, `/__blob`): no auth, not recorded.
pub(crate) fn control(_: &mut State, _: &Req) -> Option<Reply> {
    None
}

/// `POST /gitlab/api/graphql` and `POST /ghe/api/graphql`, after auth.
pub(crate) fn graphql(_: &mut State, _: Kind, _: &Req) -> Option<Reply> {
    None
}

/// `sub` below `/gitlab/api/v4/projects/:id` (`/merge_requests`, ...) of `State::projects[p]`.
pub(crate) fn gitlab(_: &mut State, _: &Req, _p: usize, _sub: &str) -> Option<Reply> {
    None
}

/// `sub` below `/ghe/api[/v3]/repos/:owner/:name` (`/pulls`, ...) of `State::projects[p]`.
pub(crate) fn ghe(_: &mut State, _: &Req, _p: usize, _sub: &str) -> Option<Reply> {
    None
}
