use std::collections::BTreeMap;

/// Folder under `src/` that holds the routes
pub const DIR: &str = "routes";

/// What a file in `src/routes/` is
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    /// A page; the string is its pattern relative to the folder (`""`, `"/about"`, `"/:id"`,
    /// `"/*"`)
    Page(String),
    Layout,
}

/// One route file after name mapping
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Route {
    /// Rust module path from the crate root, e.g. `routes::users::param_id`
    pub module: Vec<String>,
    pub kind: Kind,
    /// URL segments of the folders above (groups give none, `[id]` gives `:id`)
    pub dirs: Vec<String>,
    /// The component named after the file
    pub component: String,
    /// The file has `pub fn preload(..)`: the data the route asks for before it is built
    pub preload: bool,
}

pub(crate) fn map_name(name: &str, is_dir: bool) -> Result<(String, Option<String>), String> {
    let ident = |s: &str| s.replace('-', "_");
    if let Some(inner) = name.strip_prefix('[').and_then(|n| n.strip_suffix(']')) {
        if let Some(rest) = inner.strip_prefix("...") {
            if is_dir {
                return Err(format!(
                    "`[...{rest}]` can only be a file: a catch-all has to be last"
                ));
            }
            return check(format!("splat_{}", ident(rest)), Some(format!("*{rest}")), rest, name);
        }
        return check(format!("param_{}", ident(inner)), Some(format!(":{inner}")), inner, name);
    }
    if let Some(inner) = name.strip_prefix('(').and_then(|n| n.strip_suffix(')')) {
        if !is_dir {
            return Err(format!("`{name}` is a folder name (a group); files cannot be grouped"));
        }
        return check(format!("group_{}", ident(inner)), None, inner, name);
    }
    let m = ident(name);
    if m.is_empty()
        || !m.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        || m.starts_with(|c: char| c.is_ascii_digit())
    {
        return Err(format!(
            "`{name}` cannot be a route name (letters, digits, `-` and `_`; not starting with a digit)"
        ));
    }
    Ok((m, Some(name.to_string())))
}

fn check(
    module: String,
    seg: Option<String>,
    inner: &str,
    whole: &str,
) -> Result<(String, Option<String>), String> {
    if inner.is_empty() || !inner.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(format!("`{whole}` is not a valid route name"));
    }
    Ok((module, seg))
}

/// The name the component of the route file `stem` must have: the file's name in PascalCase, a
/// parameter by the name inside its brackets
pub fn component_name(stem: &str) -> String {
    let file = stem.rsplit('/').next().unwrap_or(stem);
    let file = file.strip_prefix('[').and_then(|f| f.strip_suffix(']')).unwrap_or(file);
    let file = file.strip_prefix("...").unwrap_or(file);
    file.split(['-', '_'])
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut c = w.chars();
            c.next().map(|f| f.to_ascii_uppercase().to_string() + c.as_str()).unwrap_or_default()
        })
        .collect()
}

/// Maps `stem` (path of the file below `src/` without `.rsx`, starting with `routes/`)
pub fn analyze(stem: &str, component: &str) -> Result<Route, String> {
    let parts: Vec<&str> = stem.split('/').collect();
    let (file, dirs) = parts[1..].split_last().ok_or("not a route file")?;
    let mut module = vec![DIR.to_string()];
    let mut segs = Vec::new();
    for d in dirs {
        let (m, seg) = map_name(d, true)?;
        module.push(m);
        segs.extend(seg);
    }
    let (m, seg) = match *file {
        "layout" => ("layout".to_string(), None),
        "index" => ("index".to_string(), None),
        "not_found" => ("not_found".to_string(), None),
        f => map_name(f, false)?,
    };
    module.push(m);
    let kind = match *file {
        "layout" => Kind::Layout,
        "index" => Kind::Page(String::new()),
        "not_found" => Kind::Page("/*".into()),
        _ => Kind::Page(format!("/{}", seg.unwrap_or_default())),
    };
    Ok(Route { module, kind, dirs: segs, component: component.to_string(), preload: false })
}

#[derive(Default)]
struct Dir {
    layout: Option<(String, Option<String>)>,
    pages: Vec<(String, String, Option<String>)>,
    dirs: BTreeMap<String, (Option<String>, Dir)>,
}

fn path_of(module: &[String], component: &str) -> String {
    format!("crate::{}::{component}", module.join("::"))
}

/// Whether the source of a route file declares `pub fn preload(target: &Target) -> Vec<Source>`
pub fn declares_preload(source: &str) -> bool {
    source.lines().any(|l| {
        let l = l.trim_start();
        l.strip_prefix("pub fn preload").is_some_and(|rest| rest.trim_start().starts_with('('))
    })
}

/// The `table()` function for these route files, placed in the `routes` module
pub fn table(routes: &[Route]) -> String {
    let mut root = Dir::default();
    for r in routes {
        let mut dir = &mut root;
        let (dirs_mod, _) = r.module[1..].split_at(r.module.len() - 2);
        // the folders' URL segments, in step with their module names
        let mut segs = r.dirs.iter();
        for m in dirs_mod {
            let seg = if m.starts_with("group_") { None } else { segs.next().cloned() };
            dir = &mut dir.dirs.entry(m.clone()).or_insert_with(|| (seg, Dir::default())).1;
        }
        let preload = r.preload.then(|| format!("crate::{}::preload", r.module.join("::")));
        match &r.kind {
            Kind::Layout => dir.layout = Some((path_of(&r.module, &r.component), preload)),
            Kind::Page(p) => dir.pages.push((p.clone(), path_of(&r.module, &r.component), preload)),
        }
    }
    fn scope(d: &Dir, prefix: &str, indent: usize, out: &mut String) {
        let pad = "    ".repeat(indent);
        for (pattern, comp, preload) in &d.pages {
            out.push_str(&format!("{pad}.at::<{comp}>({:?})\n", format!("{prefix}{pattern}")));
            if let Some(f) = preload {
                out.push_str(&format!("{pad}.preload({f})\n"));
            }
        }
        for (seg, sub) in d.dirs.values() {
            let prefix = format!("{prefix}/{}", seg.clone().unwrap_or_default());
            let prefix = prefix.trim_end_matches('/').to_string();
            match &sub.layout {
                Some((l, preload)) => {
                    out.push_str(&format!("{pad}.layout::<{l}>({prefix:?}, |r| r\n"));
                    scope(sub, "", indent + 1, out);
                    out.push_str(&format!("{pad})\n"));
                    if let Some(f) = preload {
                        out.push_str(&format!("{pad}.preload({f})\n"));
                    }
                }
                None => scope(sub, &prefix, indent, out),
            }
        }
    }
    let mut body = String::new();
    match &root.layout {
        Some((l, preload)) => {
            body.push_str(&format!("        .layout::<{l}>(\"\", |r| r\n"));
            scope(&root, "", 3, &mut body);
            body.push_str("        )\n");
            if let Some(f) = preload {
                body.push_str(&format!("        .preload({f})\n"));
            }
        }
        None => scope(&root, "", 2, &mut body),
    }
    format!(
        "\n/// The route table built from `src/routes/`.\npub fn table() -> ::yawn_router::Routes {{\n    ::yawn_router::Routes::new()\n{body}}}\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(stem: &str) -> Route {
        analyze(stem, "Page").unwrap()
    }

    #[test]
    fn maps_files_to_patterns() {
        assert_eq!(r("routes/index").kind, Kind::Page(String::new()));
        assert_eq!(r("routes/about").kind, Kind::Page("/about".into()));
        assert_eq!(r("routes/user-list").module, ["routes", "user_list"]);
        assert_eq!(r("routes/user-list").kind, Kind::Page("/user-list".into()));
        let u = r("routes/users/[id]");
        assert_eq!(
            (u.module.join("::").as_str(), &u.kind, &u.dirs[..]),
            ("routes::users::param_id", &Kind::Page("/:id".into()), &["users".to_string()][..])
        );
        assert_eq!(r("routes/docs/[...path]").kind, Kind::Page("/*path".into()));
        assert_eq!(r("routes/not_found").kind, Kind::Page("/*".into()));
        assert_eq!(r("routes/(app)/layout").kind, Kind::Layout);
        assert_eq!(r("routes/(app)/layout").dirs, Vec::<String>::new());
        assert_eq!(r("routes/[a]/[b]/x").dirs, [":a", ":b"]);
    }

    #[test]
    fn the_component_of_a_route_is_named_after_its_file() {
        assert_eq!(component_name("routes/index"), "Index");
        assert_eq!(component_name("routes/guilds/[id]/layout"), "Layout");
        assert_eq!(component_name("routes/user-list"), "UserList");
        assert_eq!(component_name("routes/not_found"), "NotFound");
        assert_eq!(component_name("routes/guilds/[id]/[module]"), "Module");
        assert_eq!(component_name("routes/docs/[...path]"), "Path");
    }

    #[test]
    fn rejects_names_that_cannot_work() {
        assert!(analyze("routes/9lives", "P").is_err());
        assert!(analyze("routes/[...rest]/x", "P").is_err());
        assert!(analyze("routes/[]", "P").is_err());
        assert!(analyze("routes/(g)", "P").is_err());
        assert!(analyze("routes/a b", "P").is_err());
    }

    #[test]
    fn table_nests_layouts_and_inlines_plain_folders() {
        let routes = [
            analyze("routes/layout", "Shell").unwrap(),
            analyze("routes/index", "Home").unwrap(),
            analyze("routes/not_found", "Missing").unwrap(),
            analyze("routes/users/index", "Users").unwrap(),
            analyze("routes/users/[id]", "User").unwrap(),
            analyze("routes/admin/layout", "AdminShell").unwrap(),
            analyze("routes/admin/index", "Admin").unwrap(),
            analyze("routes/(marketing)/pricing", "Pricing").unwrap(),
        ];
        let t = table(&routes);
        assert!(t.contains(".layout::<crate::routes::layout::Shell>(\"\", |r| r"), "{t}");
        assert!(t.contains(".at::<crate::routes::index::Home>(\"\")"), "{t}");
        assert!(t.contains(".at::<crate::routes::not_found::Missing>(\"/*\")"), "{t}");
        assert!(t.contains(".at::<crate::routes::users::index::Users>(\"/users\")"), "{t}");
        assert!(t.contains(".at::<crate::routes::users::param_id::User>(\"/users/:id\")"), "{t}");
        assert!(
            t.contains(".layout::<crate::routes::admin::layout::AdminShell>(\"/admin\", |r| r"),
            "{t}"
        );
        assert!(t.contains(".at::<crate::routes::admin::index::Admin>(\"\")"), "{t}");
        assert!(
            t.contains(".at::<crate::routes::group_marketing::pricing::Pricing>(\"/pricing\")"),
            "{t}"
        );
    }

    #[test]
    fn a_route_file_with_a_preload_hands_it_to_the_table() {
        assert!(declares_preload("pub fn preload() {\n}\ncomponent Index() {}"));
        assert!(declares_preload("    pub fn preload () { x(); }"));
        assert!(!declares_preload("fn preload() {}"));
        assert!(!declares_preload("pub fn preload_data() {}"));
        let mut layout = analyze("routes/layout", "Layout").unwrap();
        layout.preload = true;
        let mut page = analyze("routes/users/[id]", "Id").unwrap();
        page.preload = true;
        let t = table(&[layout, page, analyze("routes/index", "Index").unwrap()]);
        assert!(t.contains(".at::<crate::routes::users::param_id::Id>(\"/users/:id\")\n"), "{t}");
        assert!(t.contains(".preload(crate::routes::users::param_id::preload)"), "{t}");
        assert!(t.contains(".preload(crate::routes::layout::preload)"), "{t}");
        assert_eq!(t.matches(".preload(").count(), 2, "{t}");
    }

    #[test]
    fn without_a_root_layout_pages_sit_at_the_top() {
        let t = table(&[analyze("routes/index", "Home").unwrap()]);
        assert!(
            t.contains("Routes::new()\n        .at::<crate::routes::index::Home>(\"\")"),
            "{t}"
        );
    }
}
