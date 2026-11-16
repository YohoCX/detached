//! The read tool's executor: a query of the subset, checked against the SDL and run over a snapshot.

use std::cell::RefCell;
use std::sync::LazyLock;

use detached_core::types::{FileChange, MergeRequest, MrState, RepoInfo, RepoStatus};
use graphql_parser::query::{
    Definition, Document, OperationDefinition, Selection, SelectionSet, Value as Gql, parse_query,
};
use graphql_parser::schema::{self, ObjectType, Type, TypeDefinition, parse_schema};
use serde::Serialize;
use serde_json::{Map, Value, json};

pub const SDL: &str = include_str!("schema.graphql");
const DEPTH: usize = 8;
const SIZE: usize = 256 * 1024;

static SCHEMA: LazyLock<schema::Document<'static, &'static str>> =
    LazyLock::new(|| parse_schema(SDL).expect("the SDL parses"));

/// What a query reads, the scope already applied.
#[derive(Default)]
pub struct Snap {
    pub repos: Vec<RepoSnap>,
    pub current: Option<String>,
    pub sessions: Vec<SessionSnap>,
}

pub struct RepoSnap {
    pub info: RepoInfo,
    /// None while it loads.
    pub status: Option<Result<RepoStatus, String>>,
    /// By whether they are the open ones alone; a list not here was not asked for yet.
    pub mrs: Vec<(bool, Result<Vec<MergeRequest>, String>)>,
}

pub struct SessionSnap {
    pub id: String,
    pub provider: String,
    pub model: Option<String>,
    pub title: Option<String>,
    pub state: &'static str,
    pub repo: String,
}

/// What a field found not loaded.
#[derive(Clone, Debug, PartialEq)]
pub enum Need {
    Status(String),
    Mrs(String, bool),
}

/// The query's answer, `{data, errors?}`, and what it found not loaded; a query outside the subset
/// or the limits is refused by name.
pub fn run(snap: &Snap, query: &str) -> Result<(Value, Vec<Need>), String> {
    let doc = parse_query::<&str>(query).map_err(|e| format!("The query does not parse: {e}"))?;
    let set = check(&doc)?;
    let c = Ctx {
        snap,
        needs: RefCell::default(),
        errors: RefCell::default(),
    };
    let mut out = json!({"data": object(&c, "Query", Node::Query, set, &mut vec![])});
    let errors = c.errors.into_inner();
    if !errors.is_empty() {
        out["errors"] = errors.into();
    }
    if out.to_string().len() > SIZE {
        return Err(
            "The result is larger than 256 KB: select fewer fields or pass a limit.".into(),
        );
    }
    Ok((out, c.needs.into_inner()))
}

struct Ctx<'a> {
    snap: &'a Snap,
    needs: RefCell<Vec<Need>>,
    errors: RefCell<Vec<Value>>,
}

#[derive(Clone, Copy)]
enum Node<'a> {
    Query,
    Workspace,
    Repo(&'a RepoSnap),
    Status(&'a RepoStatus),
    File(&'a FileChange),
    Mr(&'a MergeRequest),
    Session(&'a SessionSnap),
}

enum Out<'a> {
    Value(Value),
    Node(Node<'a>),
    List(Vec<Node<'a>>),
}

type Args<'q> = [(&'q str, Gql<'q, &'q str>)];
type Resolve = for<'a, 'q> fn(&Ctx<'a>, Node<'a>, &Args<'q>) -> Result<Out<'a>, String>;

/// Every SDL field's resolver, by type and field.
const RESOLVERS: &[(&str, &str, Resolve)] = &[
    ("Query", "schema", |_, _, _| val(SDL)),
    ("Query", "workspace", |_, _, _| {
        Ok(Out::Node(Node::Workspace))
    }),
    ("Query", "repo", |c, _, a| {
        Ok(c.repo(text(a, "path"))
            .map_or(Out::Value(Value::Null), Out::Node))
    }),
    ("Query", "sessions", |c, _, a| {
        let of = |s: &&SessionSnap| text(a, "repo").is_none_or(|r| s.repo == r);
        Ok(Out::List(
            c.snap
                .sessions
                .iter()
                .filter(of)
                .map(Node::Session)
                .collect(),
        ))
    }),
    ("Workspace", "repos", |c, _, _| {
        Ok(Out::List(c.snap.repos.iter().map(Node::Repo).collect()))
    }),
    ("Workspace", "current", |c, _, _| {
        let current = c.snap.current.as_deref();
        Ok(c.repo(current).map_or(Out::Value(Value::Null), Out::Node))
    }),
    ("Repo", "path", |_, n, _| val(&n.repo().info.path)),
    ("Repo", "name", |_, n, _| val(&n.repo().info.name)),
    ("Repo", "branch", |c, n, _| val(&c.status(n.repo())?.branch)),
    ("Repo", "ahead", |c, n, _| val(c.status(n.repo())?.ahead)),
    ("Repo", "behind", |c, n, _| val(c.status(n.repo())?.behind)),
    ("Repo", "status", |c, n, _| {
        Ok(Out::Node(Node::Status(c.status(n.repo())?)))
    }),
    ("Repo", "mrs", |c, n, a| c.mrs(n.repo(), word(a, "state"))),
    ("Status", "changed", |_, n, _| val(n.status().changes.len())),
    ("Status", "staged", |_, n, _| {
        val(n
            .status()
            .changes
            .iter()
            .filter(|f| !".?".contains(f.index))
            .count())
    }),
    ("Status", "conflicted", |_, n, _| {
        val(n.status().changes.iter().filter(|f| f.conflicted).count())
    }),
    ("Status", "files", |_, n, a| {
        let limit = int(a, "limit").map_or(usize::MAX, |l| l.max(0) as usize);
        let files = n.status().changes.iter().take(limit);
        Ok(Out::List(files.map(Node::File).collect()))
    }),
    ("FileChange", "path", |_, n, _| val(&n.file().path)),
    ("FileChange", "kind", |_, n, _| val(kind(n.file()))),
    ("MergeRequest", "id", |_, n, _| {
        val(n.mr().number.to_string())
    }),
    ("MergeRequest", "title", |_, n, _| val(&n.mr().title)),
    ("MergeRequest", "state", |_, n, _| {
        val(mr_state(&n.mr().state))
    }),
    ("MergeRequest", "author", |_, n, _| val(&n.mr().author)),
    ("MergeRequest", "url", |_, n, _| val(&n.mr().web_url)),
    ("Session", "id", |_, n, _| val(&n.session().id)),
    ("Session", "provider", |_, n, _| val(&n.session().provider)),
    ("Session", "model", |_, n, _| val(&n.session().model)),
    ("Session", "title", |_, n, _| val(&n.session().title)),
    ("Session", "state", |_, n, _| val(n.session().state)),
    ("Session", "repo", |c, n, _| {
        let repo = c.repo(Some(&n.session().repo));
        Ok(repo.map_or(Out::Value(Value::Null), Out::Node))
    }),
];

fn val<'a>(v: impl Serialize) -> Result<Out<'a>, String> {
    Ok(Out::Value(json!(v)))
}

impl<'a> Node<'a> {
    fn repo(self) -> &'a RepoSnap {
        let Node::Repo(r) = self else { unreachable!() };
        r
    }
    fn status(self) -> &'a RepoStatus {
        let Node::Status(s) = self else {
            unreachable!()
        };
        s
    }
    fn file(self) -> &'a FileChange {
        let Node::File(f) = self else { unreachable!() };
        f
    }
    fn mr(self) -> &'a MergeRequest {
        let Node::Mr(m) = self else { unreachable!() };
        m
    }
    fn session(self) -> &'a SessionSnap {
        let Node::Session(s) = self else {
            unreachable!()
        };
        s
    }
}

impl<'a> Ctx<'a> {
    fn repo(&self, path: Option<&str>) -> Option<Node<'a>> {
        let r = self
            .snap
            .repos
            .iter()
            .find(|r| Some(r.info.path.as_str()) == path)?;
        Some(Node::Repo(r))
    }

    fn status(&self, r: &'a RepoSnap) -> Result<&'a RepoStatus, String> {
        match &r.status {
            Some(Ok(s)) => Ok(s),
            Some(Err(e)) => Err(format!("{}'s status: {e}", r.info.name)),
            None => {
                self.needs
                    .borrow_mut()
                    .push(Need::Status(r.info.path.clone()));
                Err(format!("{}'s status did not load in time", r.info.name))
            }
        }
    }

    fn mrs(&self, r: &'a RepoSnap, state: Option<&str>) -> Result<Out<'a>, String> {
        let open = state == Some("OPENED");
        let Some((_, listed)) = r.mrs.iter().find(|(o, _)| *o == open) else {
            let need = Need::Mrs(r.info.path.clone(), open);
            self.needs.borrow_mut().push(need);
            return Err(format!(
                "{}'s merge requests did not load in time",
                r.info.name
            ));
        };
        let listed = listed
            .as_ref()
            .map_err(|e| format!("{}'s merge requests: {e}", r.info.name))?;
        let of = |m: &&MergeRequest| state.is_none_or(|s| mr_state(&m.state) == s);
        Ok(Out::List(listed.iter().filter(of).map(Node::Mr).collect()))
    }

    fn error(&self, message: String, path: &[Value]) {
        let e = json!({"message": message, "path": path});
        self.errors.borrow_mut().push(e);
    }
}

fn kind(f: &FileChange) -> &'static str {
    let has = |c| f.index == c || f.worktree == c;
    match () {
        _ if f.conflicted => "CONFLICTED",
        _ if f.index == '?' => "UNTRACKED",
        _ if has('R') || has('C') => "RENAMED",
        _ if has('A') => "ADDED",
        _ if has('D') => "DELETED",
        _ => "MODIFIED",
    }
}

fn mr_state(s: &MrState) -> &'static str {
    match s {
        MrState::Open => "OPENED",
        MrState::Merged => "MERGED",
        MrState::Closed => "CLOSED",
    }
}

fn arg<'v, 'q>(args: &'v Args<'q>, name: &str) -> Option<&'v Gql<'q, &'q str>> {
    args.iter().find(|(n, _)| *n == name).map(|(_, v)| v)
}

fn text<'v>(args: &'v Args<'_>, name: &str) -> Option<&'v str> {
    match arg(args, name)? {
        Gql::String(s) => Some(s),
        _ => None,
    }
}

fn word<'v>(args: &'v Args<'_>, name: &str) -> Option<&'v str> {
    match arg(args, name)? {
        Gql::Enum(e) => Some(e),
        _ => None,
    }
}

fn int(args: &Args<'_>, name: &str) -> Option<i64> {
    match arg(args, name)? {
        Gql::Int(n) => n.as_i64(),
        _ => None,
    }
}

fn object<'a, 'q>(
    c: &Ctx<'a>,
    ty: &str,
    node: Node<'a>,
    set: &SelectionSet<'q, &'q str>,
    path: &mut Vec<Value>,
) -> Value {
    let mut out = Map::new();
    for f in set.items.iter().filter_map(|s| match s {
        Selection::Field(f) => Some(f),
        _ => None,
    }) {
        let key = f.alias.unwrap_or(f.name);
        path.push(key.into());
        let field = field(ty, f.name).expect("checked");
        let resolve = resolver(ty, f.name).expect("every field has a resolver");
        let v = match resolve(c, node, &f.arguments) {
            Ok(Out::Value(v)) => v,
            Ok(Out::Node(n)) => object(c, named(&field.field_type), n, &f.selection_set, path),
            Ok(Out::List(nodes)) => {
                let items = nodes.into_iter().enumerate().map(|(i, n)| {
                    path.push(i.into());
                    let v = object(c, named(&field.field_type), n, &f.selection_set, path);
                    path.pop();
                    v
                });
                items.collect()
            }
            Err(e) => {
                c.error(e, path);
                Value::Null
            }
        };
        path.pop();
        out.insert(key.into(), v);
    }
    Value::Object(out)
}

fn resolver(ty: &str, name: &str) -> Option<Resolve> {
    let r = RESOLVERS.iter().find(|(t, f, _)| *t == ty && *f == name)?;
    Some(r.2)
}

fn types() -> impl Iterator<Item = &'static TypeDefinition<'static, &'static str>> {
    SCHEMA.definitions.iter().filter_map(|d| match d {
        schema::Definition::TypeDefinition(t) => Some(t),
        _ => None,
    })
}

fn objects() -> impl Iterator<Item = &'static ObjectType<'static, &'static str>> {
    types().filter_map(|t| match t {
        TypeDefinition::Object(o) => Some(o),
        _ => None,
    })
}

fn field(ty: &str, name: &str) -> Option<&'static schema::Field<'static, &'static str>> {
    let o = objects().find(|o| o.name == ty)?;
    o.fields.iter().find(|f| f.name == name)
}

fn values(ty: &str) -> Option<Vec<&'static str>> {
    types().find_map(|t| match t {
        TypeDefinition::Enum(e) if e.name == ty => Some(e.values.iter().map(|v| v.name).collect()),
        _ => None,
    })
}

fn named<'t>(t: &Type<'t, &'t str>) -> &'t str {
    match t {
        Type::NamedType(n) => n,
        Type::ListType(t) | Type::NonNullType(t) => named(t),
    }
}

/// The query's selection, once it is in the subset and fits the SDL.
fn check<'d>(doc: &'d Document<'d, &'d str>) -> Result<&'d SelectionSet<'d, &'d str>, String> {
    let no = |what: &str| Err(format!("{what} are not supported: queries only."));
    let set = match &doc.definitions[..] {
        _ if doc
            .definitions
            .iter()
            .any(|d| matches!(d, Definition::Fragment(_))) =>
        {
            return no("Fragments");
        }
        [Definition::Operation(op)] => match op {
            OperationDefinition::SelectionSet(s) => s,
            OperationDefinition::Query(q) if !q.variable_definitions.is_empty() => {
                return no("Variables");
            }
            OperationDefinition::Query(q) if !q.directives.is_empty() => return no("Directives"),
            OperationDefinition::Query(q) => &q.selection_set,
            OperationDefinition::Mutation(_) => return no("Mutations"),
            OperationDefinition::Subscription(_) => return no("Subscriptions"),
        },
        _ => return Err("One query at a time.".into()),
    };
    if depth(set) > DEPTH {
        return Err(format!("The query is deeper than {DEPTH}."));
    }
    check_set(set, "Query")?;
    Ok(set)
}

fn depth<'q>(set: &SelectionSet<'q, &'q str>) -> usize {
    let inner = set.items.iter().map(|s| match s {
        Selection::Field(f) => depth(&f.selection_set),
        Selection::InlineFragment(f) => depth(&f.selection_set),
        Selection::FragmentSpread(_) => 0,
    });
    inner.max().map_or(0, |d| d + 1)
}

fn check_set<'q>(set: &SelectionSet<'q, &'q str>, ty: &str) -> Result<(), String> {
    let no = |what: &str| Err(format!("{what} are not supported: queries only."));
    for s in &set.items {
        let f = match s {
            Selection::Field(f) => f,
            _ => return no("Fragments"),
        };
        if !f.directives.is_empty() {
            return no("Directives");
        }
        if f.name.starts_with("__") {
            return Err("Introspection is not supported: `{ schema }` returns the SDL.".into());
        }
        let field = field(ty, f.name).ok_or_else(|| format!("{ty} has no field `{}`.", f.name))?;
        for (name, v) in &f.arguments {
            if matches!(v, Gql::Variable(_)) {
                return no("Variables");
            }
            let a = field.arguments.iter().find(|a| a.name == *name);
            let a = a.ok_or_else(|| format!("`{}` takes no argument `{name}`.", f.name))?;
            if !fits(v, &a.value_type) {
                return Err(format!("`{}`'s `{name}` is a {}.", f.name, a.value_type));
            }
        }
        for a in &field.arguments {
            let given = f.arguments.iter().any(|(n, _)| *n == a.name);
            if matches!(a.value_type, Type::NonNullType(_)) && !given {
                return Err(format!("`{}` needs `{}`.", f.name, a.name));
            }
        }
        let inner = named(&field.field_type);
        match (
            objects().any(|o| o.name == inner),
            f.selection_set.items.is_empty(),
        ) {
            (true, true) => {
                return Err(format!(
                    "`{}` needs a selection of {inner}'s fields.",
                    f.name
                ));
            }
            (true, false) => check_set(&f.selection_set, inner)?,
            (false, false) => {
                return Err(format!("`{}` is a {inner}: it takes no selection.", f.name));
            }
            (false, true) => {}
        }
    }
    Ok(())
}

fn fits<'q, 't>(v: &Gql<'q, &'q str>, ty: &Type<'t, &'t str>) -> bool {
    match (v, ty) {
        (Gql::Null, t) => !matches!(t, Type::NonNullType(_)),
        (v, Type::NonNullType(t)) => fits(v, t),
        (Gql::List(l), Type::ListType(t)) => l.iter().all(|v| fits(v, t)),
        (Gql::String(_), Type::NamedType("String" | "ID"))
        | (Gql::Int(_), Type::NamedType("Int")) => true,
        (Gql::Enum(e), Type::NamedType(n)) => values(n).is_some_and(|v| v.contains(e)),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap() -> Snap {
        let file = |path: &str, index, worktree| FileChange {
            path: path.into(),
            orig_path: None,
            index,
            worktree,
            conflicted: false,
        };
        let status = RepoStatus {
            branch: Some("main".into()),
            head: None,
            upstream: None,
            ahead: 1,
            behind: 0,
            changes: vec![file("a.rs", 'M', '.'), file("new", '?', '?')],
            last_commit: None,
            in_progress: None,
        };
        let info = RepoInfo {
            path: "/w/app".into(),
            name: "app".into(),
            folder: "/w".into(),
            worktree_of: None,
        };
        Snap {
            repos: vec![RepoSnap {
                info,
                status: Some(Ok(status)),
                mrs: vec![],
            }],
            current: Some("/w/app".into()),
            sessions: vec![],
        }
    }

    fn refused(query: &str) -> String {
        run(&snap(), query)
            .err()
            .unwrap_or_else(|| panic!("{query} ran"))
    }

    #[test]
    fn every_sdl_field_has_a_resolver_and_every_resolver_a_field() {
        for o in objects() {
            for f in &o.fields {
                assert!(resolver(o.name, f.name).is_some(), "{}.{}", o.name, f.name);
            }
        }
        for (ty, name, _) in RESOLVERS {
            assert!(field(ty, name).is_some(), "{ty}.{name} is not in the SDL");
        }
    }

    #[test]
    fn a_query_reads_the_snapshot_and_names_what_was_not_loaded() {
        let (out, needs) = run(
            &snap(),
            "{ schema workspace { current { name branch ahead status { changed staged \
                files(limit: 1) { path kind } } m: mrs(state: OPENED) { id } } } }",
        )
        .unwrap();
        assert_eq!(out["data"]["schema"], SDL);
        let current = &out["data"]["workspace"]["current"];
        assert_eq!(
            current["status"],
            json!({"changed": 2, "staged": 1, "files": [{"path": "a.rs", "kind": "MODIFIED"}]})
        );
        assert_eq!(
            (&current["branch"], &current["ahead"]),
            (&json!("main"), &json!(1))
        );
        assert_eq!(current["m"], Value::Null);
        assert_eq!(
            out["errors"][0]["path"],
            json!(["workspace", "current", "m"])
        );
        assert_eq!(needs, [Need::Mrs("/w/app".into(), true)]);
    }

    #[test]
    fn what_is_outside_the_subset_or_the_limits_is_refused_by_name() {
        for (query, said) in [
            ("{ ...F } fragment F on Query { schema }", "Fragments"),
            (
                "{ workspace { ... on Workspace { current { name } } } }",
                "Fragments",
            ),
            (
                "query($p: String!) { repo(path: $p) { name } }",
                "Variables",
            ),
            ("{ repo(path: $p) { name } }", "Variables"),
            ("{ schema @include(if: true) }", "Directives"),
            ("{ __schema { types { name } } }", "Introspection"),
            ("subscription { schema }", "Subscriptions"),
            ("mutation { schema }", "Mutations"),
            (
                "{ a { b { c { d { e { f { g { h { i } } } } } } } } }",
                "deeper than 8",
            ),
            ("{ workspace { nope } }", "Workspace has no field `nope`"),
            ("{ repo { name } }", "`repo` needs `path`"),
            (
                "{ workspace { current { mrs(state: OPEN) { id } } } }",
                "`mrs`'s `state`",
            ),
            ("{ workspace }", "needs a selection"),
            ("{ schema { x } }", "takes no selection"),
        ] {
            let e = refused(query);
            assert!(e.contains(said), "{query}: {e}");
        }
        let mut big = snap();
        let Some(Ok(status)) = &mut big.repos[0].status else {
            unreachable!()
        };
        let f = status.changes[0].clone();
        status.changes = (0..4000)
            .map(|i| FileChange {
                path: format!("{i:0>80}"),
                ..f.clone()
            })
            .collect();
        let e = run(
            &big,
            "{ workspace { repos { status { files { path } } } } }",
        )
        .unwrap_err();
        assert!(e.contains("256 KB"), "{e}");
        assert!(
            run(
                &big,
                "{ workspace { repos { status { files(limit: 10) { path } } } } }"
            )
            .is_ok()
        );
    }
}
