//! The canonical repository graph. Every subsystem, capability, donor and proof is a node with
//! a stable identity; every relation is a typed edge. Every endpoint resolves inside ECDEV's own
//! graph: there is no foreign repository to defer a reference to.

pub mod id;

pub use id::NodeId;

use crate::compact::codec::{DecodeError, Decoder, Encoder};
use crate::declare::Declaration;
use crate::formats::json::Json;
use crate::schema::{
    is_dependency, is_physical, Concept, EdgeKind, NativeStatus, NodeKind, NodeLifecycle, Scope,
};
use std::collections::{BTreeMap, BTreeSet};

pub const GRAPH_TAG: u8 = 1;

#[derive(Clone, Debug, PartialEq)]
pub struct GNode {
    pub id: NodeId,
    pub namespace: String,
    pub semantic_key: String,
    pub kind: NodeKind,
    pub concept: Concept,
    pub name: String,
    /// Physical owner: the repository whose tree holds the node (the upstream origin, for donors).
    /// Metadata only: it may change without changing the id.
    pub repository: String,
    pub canonical_path: String,
    /// Current physical location; equal to `canonical_path` once physically migrated.
    pub path: String,
    pub capabilities: Vec<String>,
    pub inputs: Vec<String>,
    pub outputs: Vec<String>,
    pub lifecycle: NodeLifecycle,
    pub native_status: NativeStatus,
    /// Proof locators that verify this node.
    pub evidence: Vec<String>,
    /// Ids of the donor nodes this node derives from.
    pub donor_lineage: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct GEdge {
    pub from: NodeId,
    pub to: NodeId,
    pub kind: EdgeKind,
    pub scope: Scope,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Graph {
    pub nodes: BTreeMap<NodeId, GNode>,
    pub edges: BTreeSet<GEdge>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GraphIssue {
    pub code: &'static str,
    pub subject: String,
    pub detail: String,
}

/// Key of the node representing the repository itself.
pub const REPOSITORY_KEY: &str = "repository";
pub use crate::schema::NAMESPACE;

/// Semantic key of a donor's capability.
pub fn capability_key(repo_id: &str, donor: &str, capability: &str) -> String {
    format!("donor-capability/{repo_id}/{donor}/{capability}")
}
/// Semantic key of an ECDEV capability: WHAT the system can do.
pub fn system_capability_key(capability: &str) -> String {
    format!("capability/{capability}")
}
/// Semantic key of an ECDEV technology: HOW the system natively does it.
pub fn technology_key(technology: &str) -> String {
    format!("technology/{technology}")
}
pub fn proof_key(repo_id: &str, locator: &str) -> String {
    format!("proof/{repo_id}/{locator}")
}
pub fn donor_local_key(repo_id: &str, donor: &str) -> String {
    format!("donor/{repo_id}/{donor}")
}

/// The `ecdev://graph/` rendering of a semantic key.
pub fn uri(namespace: &str, key: &str) -> String {
    format!("ecdev://graph/{namespace}/{}", key.replace('.', "/"))
}

/// The ontology concept a node of `kind` means unless declared otherwise.
pub fn default_concept(kind: NodeKind) -> Concept {
    match kind {
        NodeKind::Repository => Concept::Repository,
        NodeKind::Capability => Concept::Capability,
        NodeKind::Technology => Concept::Technology,
        NodeKind::Donor | NodeKind::External => Concept::Donor,
        NodeKind::Proof => Concept::Evidence,
        NodeKind::Fixture => Concept::Dataset,
        _ => Concept::Subsystem,
    }
}

impl GNode {
    pub fn new(namespace: &str, key: &str, kind: NodeKind, name: &str) -> GNode {
        GNode {
            id: NodeId::of(namespace, key),
            namespace: namespace.to_string(),
            semantic_key: key.to_string(),
            kind,
            concept: default_concept(kind),
            name: name.to_string(),
            repository: String::new(),
            canonical_path: String::new(),
            path: String::new(),
            capabilities: Vec::new(),
            inputs: Vec::new(),
            outputs: Vec::new(),
            lifecycle: NodeLifecycle::Active,
            native_status: if is_physical(kind) && kind != NodeKind::Compat {
                NativeStatus::Native
            } else if kind == NodeKind::Compat {
                NativeStatus::Compat
            } else {
                NativeStatus::Abstract
            },
            evidence: Vec::new(),
            donor_lineage: Vec::new(),
        }
    }
}

impl Graph {
    pub fn add_node(&mut self, n: GNode) -> Result<(), GraphIssue> {
        match self.nodes.get_mut(&n.id) {
            Some(existing) => join(existing, &n),
            None => {
                self.nodes.insert(n.id, n);
                Ok(())
            }
        }
    }

    pub fn add_edge(&mut self, from: NodeId, to: NodeId, kind: EdgeKind, scope: Scope) {
        self.edges.insert(GEdge {
            from,
            to,
            kind,
            scope,
        });
    }

    pub fn node_by_key(&self, namespace: &str, key: &str) -> Option<&GNode> {
        self.nodes.get(&NodeId::of(namespace, key))
    }

    pub fn inbound(&self, id: NodeId) -> impl Iterator<Item = &GEdge> {
        self.edges.iter().filter(move |e| e.to == id)
    }

    pub fn outbound(&self, id: NodeId) -> impl Iterator<Item = &GEdge> {
        self.edges.iter().filter(move |e| e.from == id)
    }

    /// Id of a declared donor: its OSS identity when the origin is known.
    pub fn donor_id(repo_id: &str, key: &str, origin: &str) -> (String, String) {
        match id::oss_key(origin) {
            Some(k) => (id::OSS_NAMESPACE.to_string(), k),
            None => (NAMESPACE.to_string(), donor_local_key(repo_id, key)),
        }
    }

    /// Builds the declared graph of the repository.
    pub fn from_declaration(d: &Declaration) -> (Graph, Vec<GraphIssue>) {
        let mut g = Graph::default();
        let mut issues = Vec::new();
        let sys = NAMESPACE;
        let repo_id = d.repository.id.as_str();
        let push = |g: &mut Graph, n: GNode, issues: &mut Vec<GraphIssue>| {
            if let Err(e) = g.add_node(n) {
                issues.push(e);
            }
        };
        let owned = |mut n: GNode| {
            n.repository = repo_id.to_string();
            n
        };
        let repo_key = format!("{REPOSITORY_KEY}/{repo_id}");
        let mut repo = owned(GNode::new(
            sys,
            &repo_key,
            NodeKind::Repository,
            &d.repository.name,
        ));
        repo.path = ".".into();
        repo.canonical_path = ".".into();
        repo.inputs = vec![d.repository.origin.clone()];
        let repo_node = repo.id;
        push(&mut g, repo, &mut issues);

        let capability = |g: &mut Graph, issues: &mut Vec<GraphIssue>, key: &str| -> NodeId {
            let n = GNode::new(sys, &system_capability_key(key), NodeKind::Capability, key);
            let id = n.id;
            if let Err(e) = g.add_node(n) {
                issues.push(e);
            }
            id
        };
        let technology_id = |key: &str| NodeId::of(sys, &technology_key(key));

        let mut seen = BTreeSet::new();
        for n in &d.repository.nodes {
            if !seen.insert(n.key.clone()) {
                issues.push(GraphIssue {
                    code: "DUPLICATE_NODE_KEY",
                    subject: n.key.clone(),
                    detail: "two nodes declare the same semantic key".into(),
                });
                continue;
            }
            if n.key.is_empty() || n.key.contains(':') || n.key.contains('/') {
                issues.push(GraphIssue {
                    code: "RESERVED_NODE_KEY",
                    subject: n.key.clone(),
                    detail: "node keys are dotted words; `:` and `/` are reserved".into(),
                });
                continue;
            }
            let mut gn = owned(GNode::new(sys, &n.key, n.kind, &n.name));
            gn.concept = n.concept;
            gn.path = n.path.clone();
            gn.canonical_path = n.canonical_path.clone();
            gn.lifecycle = n.lifecycle;
            gn.capabilities = sorted(&n.provides);
            gn.inputs = sorted(&n.inputs);
            gn.outputs = sorted(&n.outputs);
            let id = gn.id;
            push(&mut g, gn, &mut issues);
            if is_physical(n.kind) {
                g.add_edge(repo_node, id, EdgeKind::Contains, Scope::Semantic);
            }
            for c in &n.provides {
                let cid = capability(&mut g, &mut issues, c);
                g.add_edge(id, cid, EdgeKind::Provides, Scope::Semantic);
            }
            for c in &n.requires {
                let cid = capability(&mut g, &mut issues, c);
                g.add_edge(id, cid, EdgeKind::Requires, Scope::Semantic);
            }
            for t in &n.reuses {
                g.add_edge(id, technology_id(t), EdgeKind::Reuses, Scope::Semantic);
            }
        }

        let donor_ids: BTreeMap<&str, NodeId> = d
            .donors
            .iter()
            .map(|dn| {
                let (dns, dkey) = Graph::donor_id(repo_id, &dn.key, &dn.origin);
                (dn.key.as_str(), NodeId::of(&dns, &dkey))
            })
            .collect();

        for dn in &d.donors {
            let (dns, dkey) = Graph::donor_id(repo_id, &dn.key, &dn.origin);
            let mut node = GNode::new(&dns, &dkey, NodeKind::Donor, &dkey);
            node.concept = Concept::Donor;
            node.repository = dn.origin.clone();
            let did = node.id;
            push(&mut g, node, &mut issues);
            // The repository uses (or studies) the donor.
            g.add_edge(repo_node, did, EdgeKind::Uses, Scope::Semantic);
            for c in &dn.capabilities {
                let key = capability_key(repo_id, &dn.key, &c.key);
                let cap = owned(GNode::new(sys, &key, NodeKind::Capability, &c.key));
                let cid = cap.id;
                push(&mut g, cap, &mut issues);
                g.add_edge(did, cid, EdgeKind::Provides, Scope::Semantic);
                if let Some(rep) = &c.replacement {
                    let rid = NodeId::of(sys, rep);
                    g.add_edge(rid, cid, EdgeKind::Replaces, Scope::Semantic);
                    if let Some(rn) = g.nodes.get_mut(&rid) {
                        for p in &c.proofs {
                            if !rn.evidence.contains(&p.locator) {
                                rn.evidence.push(p.locator.clone());
                                rn.evidence.sort();
                            }
                        }
                    }
                }
                for p in &c.proofs {
                    let pn = owned(GNode::new(
                        sys,
                        &proof_key(repo_id, &p.locator),
                        NodeKind::Proof,
                        &p.locator,
                    ));
                    let pid = pn.id;
                    push(&mut g, pn, &mut issues);
                    g.add_edge(pid, cid, EdgeKind::Verifies, Scope::Semantic);
                }
            }
        }

        for t in &d.technologies {
            let mut tn = owned(GNode::new(
                sys,
                &technology_key(&t.key),
                NodeKind::Technology,
                &t.name,
            ));
            tn.concept = Concept::Technology;
            tn.capabilities = sorted(&t.implements);
            tn.inputs = sorted(&t.sources);
            tn.evidence = t.proofs.iter().map(|p| p.locator.clone()).collect();
            tn.evidence.sort();
            let tid = tn.id;
            push(&mut g, tn, &mut issues);
            g.add_edge(
                NodeId::of(sys, &t.node),
                tid,
                EdgeKind::Contains,
                Scope::Semantic,
            );
            for c in &t.implements {
                let cid = capability(&mut g, &mut issues, c);
                g.add_edge(tid, cid, EdgeKind::Implements, Scope::Semantic);
            }
            for p in &t.proofs {
                let pn = owned(GNode::new(
                    sys,
                    &proof_key(repo_id, &p.locator),
                    NodeKind::Proof,
                    &p.locator,
                ));
                let pid = pn.id;
                push(&mut g, pn, &mut issues);
                g.add_edge(pid, tid, EdgeKind::Verifies, Scope::Semantic);
            }
            for l in &t.lineage {
                match donor_ids.get(l.as_str()) {
                    Some(did) => g.add_edge(tid, *did, EdgeKind::LearnedFrom, Scope::Semantic),
                    None => issues.push(GraphIssue {
                        code: "UNKNOWN_LINEAGE_DONOR",
                        subject: t.key.clone(),
                        detail: format!("lineage names undeclared donor `{l}`"),
                    }),
                }
            }
            for r in &t.relations {
                g.add_edge(tid, technology_id(&r.target), r.kind, Scope::Semantic);
            }
        }

        for n in &d.repository.nodes {
            let id = NodeId::of(sys, &n.key);
            for l in &n.lineage {
                match donor_ids.get(l.as_str()) {
                    Some(did) => {
                        g.add_edge(id, *did, EdgeKind::DerivesFrom, Scope::Semantic);
                        if let Some(gn) = g.nodes.get_mut(&id) {
                            gn.donor_lineage.push(did.to_string());
                            gn.donor_lineage.sort();
                            gn.donor_lineage.dedup();
                        }
                    }
                    None => issues.push(GraphIssue {
                        code: "UNKNOWN_LINEAGE_DONOR",
                        subject: n.key.clone(),
                        detail: format!("lineage names undeclared donor `{l}`"),
                    }),
                }
            }
        }

        // Every endpoint must be a node of this graph.
        for e in &d.repository.edges {
            let resolve_end = |r: &str| -> NodeId {
                if let Some(k) = r.strip_prefix("donor:") {
                    if let Some(id) = donor_ids.get(k) {
                        return *id;
                    }
                }
                NodeId::of(sys, r)
            };
            let (from, to) = (resolve_end(&e.from), resolve_end(&e.to));
            g.add_edge(from, to, e.kind, e.scope);
        }
        issues.extend(g.validate());
        (g, issues)
    }

    /// Ids referenced by edges but not present (always empty for a valid graph).
    pub fn unresolved(&self) -> BTreeSet<NodeId> {
        self.edges
            .iter()
            .flat_map(|e| [e.from, e.to])
            .filter(|id| !self.nodes.contains_key(id))
            .collect()
    }

    /// Edges pointing at `id`: backlinks are derived, never stored.
    pub fn backlinks(&self, id: NodeId) -> Vec<&GEdge> {
        self.edges.iter().filter(|e| e.to == id).collect()
    }

    /// Structural validity: endpoints, edge typing and dependency acyclicity.
    pub fn validate(&self) -> Vec<GraphIssue> {
        let mut out = Vec::new();
        for e in &self.edges {
            let (from, to) = (self.nodes.get(&e.from), self.nodes.get(&e.to));
            let (Some(from), Some(to)) = (from, to) else {
                out.push(GraphIssue {
                    code: "DANGLING_EDGE",
                    subject: format!("{} -{}-> {}", e.from, e.kind, e.to),
                    detail: "an edge endpoint is not a node of the ECDEV graph".into(),
                });
                continue;
            };
            if let Some(why) = edge_illegal(e.kind, e.scope, from.kind, to.kind) {
                out.push(GraphIssue {
                    code: "ILLEGAL_EDGE",
                    subject: format!("{} -{}-> {}", from.semantic_key, e.kind, to.semantic_key),
                    detail: why,
                });
            }
        }
        if let Some(cycle) = self.dependency_cycle() {
            out.push(GraphIssue {
                code: "DEPENDENCY_CYCLE",
                subject: cycle.join(" -> "),
                detail: "DEPENDS_ON edges among physical nodes must be acyclic".into(),
            });
        }
        out
    }

    fn dependency_cycle(&self) -> Option<Vec<String>> {
        let mut adj: BTreeMap<NodeId, Vec<NodeId>> = BTreeMap::new();
        for e in &self.edges {
            if e.kind == EdgeKind::DependsOn && e.from != e.to {
                adj.entry(e.from).or_default().push(e.to);
            }
        }
        // 0 = unvisited, 1 = on stack, 2 = done
        let mut state: BTreeMap<NodeId, u8> = BTreeMap::new();
        let mut stack: Vec<NodeId> = Vec::new();
        fn dfs(
            n: NodeId,
            adj: &BTreeMap<NodeId, Vec<NodeId>>,
            state: &mut BTreeMap<NodeId, u8>,
            stack: &mut Vec<NodeId>,
        ) -> Option<Vec<NodeId>> {
            state.insert(n, 1);
            stack.push(n);
            for m in adj.get(&n).into_iter().flatten() {
                match state.get(m).copied().unwrap_or(0) {
                    1 => {
                        let pos = stack.iter().position(|x| x == m).unwrap();
                        let mut c = stack[pos..].to_vec();
                        c.push(*m);
                        return Some(c);
                    }
                    0 => {
                        if let Some(c) = dfs(*m, adj, state, stack) {
                            return Some(c);
                        }
                    }
                    _ => {}
                }
            }
            stack.pop();
            state.insert(n, 2);
            None
        }
        for n in adj.keys() {
            if state.get(n).copied().unwrap_or(0) == 0 {
                if let Some(c) = dfs(*n, &adj, &mut state, &mut stack) {
                    return Some(
                        c.iter()
                            .map(|id| {
                                self.nodes
                                    .get(id)
                                    .map(|n| n.semantic_key.clone())
                                    .unwrap_or_else(|| id.to_string())
                            })
                            .collect(),
                    );
                }
            }
        }
        None
    }

    /// Merges another graph into this one. Commutative and associative; identities are never
    /// rewritten. Two nodes with the same id must agree on kind.
    pub fn merge(&mut self, other: &Graph) -> Vec<GraphIssue> {
        let mut issues = Vec::new();
        for n in other.nodes.values() {
            if let Err(e) = self.add_node(n.clone()) {
                issues.push(e);
            }
        }
        self.edges.extend(other.edges.iter().cloned());
        issues
    }

    // -----------------------------------------------------------------------------------------
    // Serialization.

    pub fn encode(&self) -> Vec<u8> {
        let mut e = Encoder::new(GRAPH_TAG);
        e.u64(self.nodes.len() as u64);
        for n in self.nodes.values() {
            e.bytes(&n.id.0)
                .str(&n.namespace)
                .str(&n.semantic_key)
                .u8(n.kind.rank())
                .u8(n.concept.rank())
                .str(&n.name)
                .str(&n.repository)
                .str(&n.canonical_path)
                .str(&n.path)
                .strs(&n.capabilities)
                .strs(&n.inputs)
                .strs(&n.outputs)
                .u8(n.lifecycle.rank())
                .u8(n.native_status.rank())
                .strs(&n.evidence)
                .strs(&n.donor_lineage);
        }
        e.u64(self.edges.len() as u64);
        for x in &self.edges {
            e.bytes(&x.from.0)
                .bytes(&x.to.0)
                .u8(x.kind.rank())
                .u8(x.scope.rank());
        }
        e.finish()
    }

    pub fn decode(bytes: &[u8]) -> Result<Graph, DecodeError> {
        let mut d = Decoder::open(bytes, GRAPH_TAG)?;
        let mut g = Graph::default();
        let id16 = |d: &mut Decoder| -> Result<NodeId, DecodeError> {
            let b = d.bytes()?;
            let arr: [u8; 16] = b
                .try_into()
                .map_err(|_| DecodeError("node id is not 16 bytes".into()))?;
            Ok(NodeId(arr))
        };
        for _ in 0..d.u64()? {
            let id = id16(&mut d)?;
            let n = GNode {
                id,
                namespace: d.str()?,
                semantic_key: d.str()?,
                kind: d.word(NodeKind::ALL)?,
                concept: d.word(Concept::ALL)?,
                name: d.str()?,
                repository: d.str()?,
                canonical_path: d.str()?,
                path: d.str()?,
                capabilities: d.strs()?,
                inputs: d.strs()?,
                outputs: d.strs()?,
                lifecycle: d.word(NodeLifecycle::ALL)?,
                native_status: d.word(NativeStatus::ALL)?,
                evidence: d.strs()?,
                donor_lineage: d.strs()?,
            };
            if NodeId::of(&n.namespace, &n.semantic_key) != id {
                return Err(DecodeError(format!(
                    "node {id} does not match its namespace and key"
                )));
            }
            g.nodes.insert(id, n);
        }
        for _ in 0..d.u64()? {
            let from = id16(&mut d)?;
            let to = id16(&mut d)?;
            g.edges.insert(GEdge {
                from,
                to,
                kind: d.word(EdgeKind::ALL)?,
                scope: d.word(Scope::ALL)?,
            });
        }
        d.end()?;
        Ok(g)
    }

    /// The deterministic human view.
    pub fn render_text(&self) -> String {
        let mut s = String::new();
        let label = |id: &NodeId| -> String {
            self.nodes
                .get(id)
                .map(|n| format!("{}:{}", n.namespace, n.semantic_key))
                .unwrap_or_else(|| id.to_string())
        };
        for n in self.nodes.values() {
            s.push_str(&format!(
                "node {} {} {}:{} path={} canonical={} lifecycle={} native={}\n",
                n.id,
                n.kind,
                n.namespace,
                n.semantic_key,
                if n.path.is_empty() { "-" } else { &n.path },
                if n.canonical_path.is_empty() {
                    "-"
                } else {
                    &n.canonical_path
                },
                n.lifecycle,
                n.native_status
            ));
        }
        for e in &self.edges {
            s.push_str(&format!(
                "edge {} -{}/{}-> {}\n",
                label(&e.from),
                e.kind,
                e.scope,
                label(&e.to)
            ));
        }
        s
    }

    pub fn to_json(&self) -> Json {
        let nodes = self
            .nodes
            .values()
            .map(|n| {
                Json::obj()
                    .with("id", n.id.to_string())
                    .with("namespace", &n.namespace)
                    .with("semantic_key", &n.semantic_key)
                    .with("kind", n.kind.wire())
                    .with("concept", n.concept.wire())
                    .with("uri", uri(&n.namespace, &n.semantic_key))
                    .with("name", &n.name)
                    .with("repository", &n.repository)
                    .with("canonical_path", &n.canonical_path)
                    .with("path", &n.path)
                    .with("capabilities", n.capabilities.clone())
                    .with("inputs", n.inputs.clone())
                    .with("outputs", n.outputs.clone())
                    .with("lifecycle", n.lifecycle.wire())
                    .with("native_status", n.native_status.wire())
                    .with("evidence", n.evidence.clone())
                    .with("donor_lineage", n.donor_lineage.clone())
            })
            .collect::<Vec<_>>();
        let edges = self
            .edges
            .iter()
            .map(|e| {
                Json::obj()
                    .with("from", e.from.to_string())
                    .with("to", e.to.to_string())
                    .with("kind", e.kind.wire())
                    .with("scope", e.scope.wire())
            })
            .collect::<Vec<_>>();
        Json::obj()
            .with("schema", crate::protocol::schema_identity())
            .with("nodes", Json::Array(nodes))
            .with("edges", Json::Array(edges))
    }
}

fn sorted(v: &[String]) -> Vec<String> {
    let mut v = v.to_vec();
    v.sort();
    v.dedup();
    v
}

/// Commutative join of two descriptions of the same identity.
fn join(a: &mut GNode, b: &GNode) -> Result<(), GraphIssue> {
    if a.kind != b.kind {
        return Err(GraphIssue {
            code: "IDENTITY_KIND_CONFLICT",
            subject: a.id.to_string(),
            detail: format!(
                "`{}:{}` is {} in one graph and {} in another",
                a.namespace, a.semantic_key, a.kind, b.kind
            ),
        });
    }
    let min = |x: &mut String, y: &String| {
        if x.is_empty() || (!y.is_empty() && y < x) {
            *x = y.clone();
        }
    };
    min(&mut a.name, &b.name);
    min(&mut a.repository, &b.repository);
    min(&mut a.canonical_path, &b.canonical_path);
    min(&mut a.path, &b.path);
    for (x, y) in [
        (&mut a.capabilities, &b.capabilities),
        (&mut a.inputs, &b.inputs),
        (&mut a.outputs, &b.outputs),
        (&mut a.evidence, &b.evidence),
        (&mut a.donor_lineage, &b.donor_lineage),
    ] {
        x.extend(y.iter().cloned());
        x.sort();
        x.dedup();
    }
    a.lifecycle = a.lifecycle.max(b.lifecycle);
    a.native_status = a.native_status.max(b.native_status);
    Ok(())
}

/// Returns why an edge of `kind`/`scope` from `from` to `to` is illegal, if it is.
pub fn edge_illegal(kind: EdgeKind, scope: Scope, from: NodeKind, to: NodeKind) -> Option<String> {
    use NodeKind as K;
    let dependency_scope = !matches!(scope, Scope::Semantic);
    if is_dependency(kind) != dependency_scope {
        return Some(if is_dependency(kind) {
            format!("{kind} needs a dependency scope, not SEMANTIC")
        } else {
            format!("{kind} is a semantic relation; its scope must be SEMANTIC")
        });
    }
    if scope == Scope::Architectural && kind != EdgeKind::DependsOn {
        return Some("only DEPENDS_ON may be ARCHITECTURAL".into());
    }
    let phys = |k: NodeKind| is_physical(k) && !matches!(k, K::Repository);
    let tech = |k: NodeKind| k == K::Technology;
    let ok = match kind {
        EdgeKind::DependsOn | EdgeKind::Calls => {
            // Code that belongs to no node is attributed to its repository.
            (phys(from) || from == K::Repository)
                && (phys(to) || matches!(to, K::External | K::Donor))
        }
        EdgeKind::Provides => (phys(from) || from == K::Donor) && to == K::Capability,
        EdgeKind::Consumes | EdgeKind::Exposes | EdgeKind::Requires => {
            (phys(from) || from == K::Repository) && to == K::Capability
        }
        EdgeKind::Implements => (tech(from) || phys(from)) && to == K::Capability,
        EdgeKind::Reads | EdgeKind::Writes | EdgeKind::Emits | EdgeKind::Subscribes => {
            phys(from) && (phys(to) || matches!(to, K::Capability | K::Donor))
        }
        EdgeKind::Controls | EdgeKind::Owns => phys(from) && (phys(to) || to == K::Capability),
        EdgeKind::Produces => phys(from) && phys(to),
        EdgeKind::Replaces | EdgeKind::Supersedes => {
            (phys(from) || tech(from)) && (phys(to) || matches!(to, K::Capability | K::Technology))
        }
        EdgeKind::Evolves
        | EdgeKind::Specializes
        | EdgeKind::Generalizes
        | EdgeKind::AlternativeFor
        | EdgeKind::ForkedFrom
        | EdgeKind::Merges => (tech(from) && tech(to)) || (phys(from) && phys(to)),
        EdgeKind::Reuses => phys(from) && tech(to),
        EdgeKind::DerivesFrom => (phys(from) || from == K::Capability) && to == K::Donor,
        EdgeKind::LearnedFrom => (tech(from) || phys(from)) && to == K::Donor,
        EdgeKind::Verifies => {
            matches!(from, K::Proof | K::Test)
                && (phys(to) || matches!(to, K::Capability | K::Technology))
        }
        EdgeKind::Contains => {
            (from == K::Repository || phys(from))
                && (phys(to) || matches!(to, K::Capability | K::Proof | K::Technology))
        }
        EdgeKind::Uses => from == K::Repository && to == K::Donor,
        EdgeKind::AuthorizedBy => phys(from) && (phys(to) || to == K::Capability),
    };
    (!ok).then(|| format!("{kind} may not connect {from} to {to}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(ns: &str, key: &str, kind: NodeKind) -> GNode {
        let mut n = GNode::new(ns, key, kind, key);
        n.path = key.replace('.', "/");
        n
    }

    #[test]
    fn merge_is_commutative_and_keeps_ids() {
        let mut a = Graph::default();
        a.add_node(node("a", "core", NodeKind::Kernel)).unwrap();
        a.add_node(node("oss", "github.com/x/y", NodeKind::Donor))
            .unwrap();
        let mut b = Graph::default();
        b.add_node(node("b", "core", NodeKind::Kernel)).unwrap();
        let mut d = node("oss", "github.com/x/y", NodeKind::Donor);
        d.name = "y".into();
        b.add_node(d).unwrap();
        b.add_edge(
            NodeId::of("b", "core"),
            NodeId::of("oss", "github.com/x/y"),
            EdgeKind::DependsOn,
            Scope::Runtime,
        );
        let mut ab = a.clone();
        assert!(ab.merge(&b).is_empty());
        let mut ba = b.clone();
        assert!(ba.merge(&a).is_empty());
        assert_eq!(ab, ba);
        assert_eq!(ab.nodes.len(), 3, "the shared donor is one node");
        assert!(ab.nodes.contains_key(&NodeId::of("a", "core")));
        assert!(ab.nodes.contains_key(&NodeId::of("b", "core")));
    }

    #[test]
    fn kind_conflict_is_reported() {
        let mut a = Graph::default();
        a.add_node(node("a", "x", NodeKind::Kernel)).unwrap();
        let mut b = Graph::default();
        b.add_node(node("a", "x", NodeKind::Tool)).unwrap();
        assert_eq!(a.merge(&b)[0].code, "IDENTITY_KIND_CONFLICT");
    }

    #[test]
    fn binary_roundtrip_and_tamper_detection() {
        let mut g = Graph::default();
        g.add_node(node("a", "core", NodeKind::Kernel)).unwrap();
        g.add_node(node("a", "substrate.store", NodeKind::Substrate))
            .unwrap();
        g.add_edge(
            NodeId::of("a", "substrate.store"),
            NodeId::of("a", "core"),
            EdgeKind::DependsOn,
            Scope::Runtime,
        );
        let bytes = g.encode();
        assert_eq!(Graph::decode(&bytes).unwrap(), g);
        assert_eq!(g.encode(), Graph::decode(&bytes).unwrap().encode());
        // Changing a key without its id is detected.
        let mut tampered = bytes.clone();
        let pos = tampered.windows(4).position(|w| w == b"core").unwrap();
        tampered[pos] = b'k';
        assert!(Graph::decode(&tampered).is_err());
    }

    #[test]
    fn edge_typing_and_cycles() {
        assert!(edge_illegal(
            EdgeKind::DependsOn,
            Scope::Semantic,
            NodeKind::Kernel,
            NodeKind::Kernel
        )
        .is_some());
        assert!(edge_illegal(
            EdgeKind::Verifies,
            Scope::Semantic,
            NodeKind::Kernel,
            NodeKind::Kernel
        )
        .is_some());
        assert!(edge_illegal(
            EdgeKind::Verifies,
            Scope::Semantic,
            NodeKind::Proof,
            NodeKind::Capability
        )
        .is_none());
        let mut g = Graph::default();
        for k in ["a", "b"] {
            g.add_node(node("n", k, NodeKind::Substrate)).unwrap();
        }
        g.add_edge(
            NodeId::of("n", "a"),
            NodeId::of("n", "b"),
            EdgeKind::DependsOn,
            Scope::Runtime,
        );
        g.add_edge(
            NodeId::of("n", "b"),
            NodeId::of("n", "a"),
            EdgeKind::DependsOn,
            Scope::Runtime,
        );
        assert!(g.validate().iter().any(|i| i.code == "DEPENDENCY_CYCLE"));
    }

    #[test]
    fn single_repository_edge_legality() {
        use EdgeKind as E;
        use NodeKind as K;
        let legal = |e: EdgeKind, f: NodeKind, t: NodeKind| {
            edge_illegal(e, Scope::Semantic, f, t).is_none()
        };
        assert!(legal(E::Uses, K::Repository, K::Donor));
        assert!(!legal(E::Uses, K::Domain, K::Donor));
        assert!(!legal(E::Uses, K::Repository, K::Technology));
        assert!(legal(E::AuthorizedBy, K::Adapter, K::Capability));
        assert!(legal(E::Contains, K::Repository, K::Domain));
        assert!(!legal(E::Contains, K::Domain, K::Repository));
        assert!(edge_illegal(E::Uses, Scope::Runtime, K::Repository, K::Donor).is_some());
    }

    #[test]
    fn an_unresolved_endpoint_is_dangling() {
        let mut g = Graph::default();
        g.add_node(node("ecdev", "a", NodeKind::Domain)).unwrap();
        g.add_edge(
            NodeId::of("ecdev", "a"),
            NodeId::of("ecdev", "elsewhere"),
            EdgeKind::DependsOn,
            Scope::Runtime,
        );
        assert!(g.validate().iter().any(|i| i.code == "DANGLING_EDGE"));
    }
}
