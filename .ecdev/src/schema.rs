//! The closed vocabularies of ECDEV governance. Every declaration uses exactly these words. The rendered form of these tables is part of the
//! schema identity (see `protocol`), so any change is a protocol change.

macro_rules! vocabulary {
    ($(#[$m:meta])* $name:ident { $($v:ident = $wire:literal : $meaning:literal),* $(,)? }) => {
        $(#[$m])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub enum $name { $($v),* }
        impl $name {
            pub const ALL: &'static [$name] = &[$($name::$v),*];
            /// The UPPER_SNAKE protocol word, used in every rendered view.
            pub fn wire(self) -> &'static str { match self { $($name::$v => $wire),* } }
            /// The Rust variant name, used in declarations.
            pub fn variant(self) -> &'static str { match self { $($name::$v => stringify!($v)),* } }
            pub fn meaning(self) -> &'static str { match self { $($name::$v => $meaning),* } }
            pub fn from_variant(s: &str) -> Option<Self> {
                match s { $(stringify!($v) => Some($name::$v),)* _ => None }
            }
            pub fn from_wire(s: &str) -> Option<Self> {
                match s { $($wire => Some($name::$v),)* _ => None }
            }
            pub fn rank(self) -> u8 { self as u8 }
        }
        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str(self.wire()) }
        }
    };
}

vocabulary! {
    /// What a graph node is structurally. Physical kinds own a canonical directory; abstract
    /// kinds do not. What a node MEANS is its `Concept`.
    NodeKind {
        Repository = "REPOSITORY": "the ECDEV repository itself",
        Kernel = "KERNEL": "domain-free primitives every other plane may use (core/)",
        Substrate = "SUBSTRATE": "a cross-domain machinery owner: storage, execution, events, projection (substrate/<name>)",
        Domain = "DOMAIN": "domain semantics; never depends on adapters (domain/<name>)",
        Adapter = "ADAPTER": "the boundary to the outside world: protocols, providers, engines (adapter/<name>)",
        Application = "APPLICATION": "an entry point or user interface; nothing depends on it (apps/<name>)",
        Tool = "TOOL": "construction tooling that never ships (tools/<name>)",
        Test = "TEST": "a verification suite (tests/ or <node>/tests)",
        Fixture = "FIXTURE": "frozen test data, including frozen oracle outputs (tests/fixtures/<name>)",
        Research = "RESEARCH": "non-authoritative study code; never on a build path (research/<name>)",
        Compat = "COMPAT": "a migration shim with an expiry condition (compat/<name>)",
        Governance = "GOVERNANCE": "the ECDEV governance subsystem (.ecdev)",
        Capability = "CAPABILITY": "WHAT ECDEV can do: an abstract behaviour, provided and required by nodes",
        Technology = "TECHNOLOGY": "HOW ECDEV natively does it: a canonical native mechanism implementing capabilities",
        Donor = "DONOR": "an external OSS technology under the donor lifecycle",
        External = "EXTERNAL": "an observed external package that is not (yet) a registered donor",
        Proof = "PROOF": "an executable proof: a parity or regression test",
    }
}

vocabulary! {
    /// The ECDEV ontology: what a node means, whatever repository it lives in.
    Concept {
        System = "SYSTEM": "the whole of ECDEV",
        Repository = "REPOSITORY": "the repository",
        Subsystem = "SUBSYSTEM": "a cohesive owner of responsibility",
        Node = "NODE": "a participant in a distributed deployment",
        Symbol = "SYMBOL": "a named code item",
        Interface = "INTERFACE": "a public surface",
        Capability = "CAPABILITY": "what the system can do",
        Technology = "TECHNOLOGY": "how the system natively does it",
        Service = "SERVICE": "a long-running provider of capabilities",
        Runtime = "RUNTIME": "execution machinery",
        Resource = "RESOURCE": "something that can be owned, allocated or addressed",
        Entity = "ENTITY": "an identified thing of the world model",
        Agent = "AGENT": "an actor that decides",
        Machine = "MACHINE": "a physical or simulated machine",
        Dataset = "DATASET": "a body of data",
        Protocol = "PROTOCOL": "a rule set for communication",
        Transport = "TRANSPORT": "moving bytes between places",
        Store = "STORE": "durable state",
        Model = "MODEL": "a learned or analytic model",
        Policy = "POLICY": "a rule constraining behaviour",
        Authority = "AUTHORITY": "who may do what",
        Event = "EVENT": "something that happened",
        Command = "COMMAND": "a request to change state",
        Observation = "OBSERVATION": "something perceived",
        Decision = "DECISION": "a choice made with reasons",
        State = "STATE": "a value at a point in logical time",
        Evidence = "EVIDENCE": "proof that a claim holds",
        Donor = "DONOR": "external technology studied",
        Artifact = "ARTIFACT": "a produced build or data output",
        Contract = "CONTRACT": "a versioned interface agreement",
        Experiment = "EXPERIMENT": "a controlled trial",
        Benchmark = "BENCHMARK": "a measured workload",
        Evaluation = "EVALUATION": "a judged trial or verification suite",
    }
}

vocabulary! {
    /// Typed relations. One word per relation; inverses (BELONGS_TO, depended-on-by,
    /// backlinks) are derived, never stored.
    EdgeKind {
        Contains = "CONTAINS": "structural containment",
        DependsOn = "DEPENDS_ON": "the source cannot build or run without the target (native build/run dependency, scoped)",
        Implements = "IMPLEMENTS": "the source technology or node implements the target capability or contract",
        Provides = "PROVIDES": "the source offers the target capability to ECDEV",
        Consumes = "CONSUMES": "the source uses the target capability",
        Requires = "REQUIRES": "the source needs the target capability at run time, from whichever node provides it",
        Calls = "CALLS": "the source invokes the target at run time",
        Reads = "READS": "the source reads the target's state",
        Writes = "WRITES": "the source writes the target's state",
        Emits = "EMITS": "the source produces the target event",
        Subscribes = "SUBSCRIBES": "the source reacts to the target event",
        Controls = "CONTROLS": "the source governs the target's behaviour or lifecycle",
        Owns = "OWNS": "the source is the single authority over the target",
        Exposes = "EXPOSES": "the source offers the target as a public surface",
        Produces = "PRODUCES": "the source produces the target artifact",
        Replaces = "REPLACES": "the source is the native replacement of the target",
        Supersedes = "SUPERSEDES": "the source retires the target",
        Evolves = "EVOLVES": "the source is the next generation of the target technology",
        Specializes = "SPECIALIZES": "the source is a legitimate specialization of the target",
        Generalizes = "GENERALIZES": "the source generalizes the target",
        AlternativeFor = "ALTERNATIVE_FOR": "the source is a declared alternative implementation of the target",
        ForkedFrom = "FORKED_FROM": "the source diverged from the target",
        Merges = "MERGES": "the source unifies the target into itself",
        Reuses = "REUSES": "the source compiles the target technology in natively (materialized source; no service)",
        Verifies = "VERIFIES": "the source proves the target's behaviour",
        DerivesFrom = "DERIVES_FROM": "provenance: the source was learned from the target donor",
        LearnedFrom = "LEARNED_FROM": "the source's mechanism was learned from the target (knowledge lineage)",
        Uses = "USES": "the repository uses or studies the target donor (one edge per declared donor)",
        AuthorizedBy = "AUTHORIZED_BY": "the authority path: the source action or actor is authorized by the target capability or node",
    }
}

vocabulary! {
    /// How an edge binds. DEPENDS_ON and CALLS carry a dependency scope; ARCHITECTURAL marks a
    /// logical dependency with no build or run coupling; every other edge is SEMANTIC.
    Scope {
        Runtime = "RUNTIME": "linked into or executed by shipped artifacts",
        Build = "BUILD": "executed while building (build scripts, code generators, build tools)",
        Linked = "LINKED": "a native library or FFI symbol linked into an artifact",
        Test = "TEST": "executed only by tests (dev-dependencies, oracles)",
        Architectural = "ARCHITECTURAL": "a logical dependency of the system graph; no code coupling",
        Semantic = "SEMANTIC": "a relation that is not a dependency",
    }
}

vocabulary! {
    /// What kind of mechanism a technology is.
    TechnologyKind {
        Algorithm = "ALGORITHM": "a procedure",
        DataStructure = "DATA_STRUCTURE": "an organization of data",
        Protocol = "PROTOCOL": "a communication mechanism",
        Codec = "CODEC": "an encoding or serialization",
        Storage = "STORAGE": "a persistence mechanism",
        Runtime = "RUNTIME": "an execution mechanism",
        Parser = "PARSER": "a language or format reader",
        Primitive = "PRIMITIVE": "a foundational building block",
    }
}

vocabulary! {
    /// THE technology lifecycle. Effective states are computed; a claim above them is false green.
    TechnologyLifecycle {
        Idea = "IDEA": "named, no code",
        Experimental = "EXPERIMENTAL": "code exists in a node",
        Native = "NATIVE": "the implementing node is native: no donor, no external package",
        Proven = "PROVEN": "fresh passing proofs of every declared proof",
        Canonical = "CANONICAL": "proven, declared canonical, and no unrelated duplicate in the repository",
                Superseded = "SUPERSEDED": "an evolution or replacement exists; kept for lineage",
    }
}

vocabulary! {
    /// Dimensions an improvement claim may be about.
    Dimension {
        Correctness = "CORRECTNESS": "", Latency = "LATENCY": "", Throughput = "THROUGHPUT": "",
        Memory = "MEMORY": "", Disk = "DISK": "", Startup = "STARTUP": "", Determinism = "DETERMINISM": "",
        Portability = "PORTABILITY": "", FailureRecovery = "FAILURE_RECOVERY": "", Security = "SECURITY": "",
        Simplicity = "SIMPLICITY": "", DependencyCount = "DEPENDENCY_COUNT": "", BinarySize = "BINARY_SIZE": "",
        CompileTime = "COMPILE_TIME": "", Energy = "ENERGY": "", Scalability = "SCALABILITY": "",
        OperationalComplexity = "OPERATIONAL_COMPLEXITY": "",
    }
}

vocabulary! {
    /// Lifecycle of a repository node (not of donors).
    NodeLifecycle {
        Planned = "PLANNED": "declared target; no physical tree yet",
        Active = "ACTIVE": "exists and participates",
        Deprecated = "DEPRECATED": "exists, scheduled for removal; must name its replacement",
        Retired = "RETIRED": "tombstone: identity is kept forever, the tree is gone",
    }
}

vocabulary! {
    /// THE donor lifecycle. Effective states are computed
    /// from evidence, declared states are only claims.
    DonorState {
        Discovered = "DISCOVERED": "the technology is known to exist or is observed in the tree",
        Registered = "REGISTERED": "declared with origin and licence; counts in every denominator forever",
        Censused = "CENSUSED": "decomposed into at least one capability",
        TechnologyMapped = "TECHNOLOGY_MAPPED": "every required capability maps to a capability or technology of the canonical graph",
        Specified = "SPECIFIED": "every required capability has a specification",
        NativeTargeted = "NATIVE_TARGETED": "every required capability names a declared native replacement node (it may be PLANNED: targeted, not yet shadowing)",
        NativeShadow = "NATIVE_SHADOW": "every required capability has an existing native replacement node",
        ParityProven = "PARITY_PROVEN": "every replacement is native and every required capability has fresh passing parity proofs",
        RelevanceResolved = "CAPABILITY_RELEVANCE_RESOLVED": "every declared capability's production adoption is decided: relied on by ECDEV (and required) or not adopted with a review (and not required); research value is the separate knowledge dimension",
        Cutover = "CUTOVER": "cutover declared, no runtime or linked edge to the donor remains, regression proven",
        Extinct = "EXTINCT": "every extinction gate holds",
    }
}

vocabulary! {
    /// What a record of a repository's universe is: everything the repository knows about, of
    /// which declared donors are only the part under the donor lifecycle.
    UniverseKind {
        OssRepository = "OSS_REPOSITORY": "an open-source code repository",
        InformationSource = "INFORMATION_SOURCE": "a data, market, news, document or open-source-intelligence source",
        OfficialAuthority = "OFFICIAL_AUTHORITY": "a regulator, registry, statistics office or other official body",
        ProtocolSpec = "PROTOCOL_SPEC": "a wire protocol or interface specification",
        AcademicReference = "ACADEMIC_REFERENCE": "an academic reference: article, survey, thesis or course",
        Benchmark = "BENCHMARK": "a benchmark suite or measured workload",
        Dataset = "DATASET": "a dataset usable as fixture or training material",
        Simulator = "SIMULATOR": "a simulator or co-simulation contract",
        Standard = "STANDARD": "a standard (ISO, IEC, IEEE, W3C, ...)",
        ReferenceArchitecture = "REFERENCE_ARCHITECTURE": "a reference architecture or thing/twin model",
        Paper = "PAPER": "a paper whose algorithm or semantics is implemented or mapped",
    }
}

vocabulary! {
    /// The universe-record states: the layers below the donor ladder's CENSUSED. Effective
    /// states are computed from the record; above SELECTED_FOR_CENSUS a record's upstream is a
    /// declared donor and its state is the donor ladder's.
    UniverseState {
        Discovered = "DISCOVERED": "the record exists in the repository's universe",
        Registered = "REGISTERED": "its record claims registration and its origin resolves to a global donor identity",
        RelevanceResolved = "RELEVANCE_RESOLVED": "registered, and its relevance to the repository is decided (relevant with a reason, or not relevant with a reason)",
        SelectedForCensus = "SELECTED_FOR_CENSUS": "relevant and selected for deep census: cloned or pinned at a revision, or materialized as a declared donor",
    }
}

vocabulary! {
    /// What ECDEV has learned from a donor, independent of runtime adoption.
    KnowledgeState {
        Unreviewed = "UNREVIEWED": "no capability has been extracted from the donor",
        StructuralCensus = "STRUCTURAL_CENSUS": "capabilities are extracted but some have no knowledge decision yet (or a known capability was withdrawn)",
        SemanticCensus = "SEMANTIC_CENSUS": "every extracted capability has a knowledge decision and none is open, but the donor's whole-source semantic review is not COMPLETE: capabilities may remain unextracted",
        ActiveStudy = "ACTIVE_STUDY": "every capability is semantically reviewed and open study, benchmark or algorithm questions remain",
        StudyComplete = "STUDY_COMPLETE": "the donor's whole-source semantic review is COMPLETE and every capability's knowledge is resolved (absorbed, independent, divergent, reference only, or no research value on an admissible ground)",
    }
}

vocabulary! {
    /// Exceptional donor states and their exact semantics.
    ExceptionKind {
        Blocked = "BLOCKED": "progress halted by a named blocker; the donor keeps its effective state and stays in every denominator",
        Rejected = "REJECT_RUNTIME": "the donor's runtime implementation was evaluated and never adopted; legal only with zero observed edges now and in all census history. Says nothing about knowledge: the donor stays in the knowledge universe and every capability keeps its own knowledge status",
        Superseded = "SUPERSEDED": "merged into another registered donor that covers all of its capabilities; the successor carries its edges",
    }
}

vocabulary! {
    /// Evidence classes.
    ProofKind {
        Parity = "PARITY": "the native replacement behaves as the donor on the capability's specified cases",
        Regression = "REGRESSION": "the native replacement's own behaviour is pinned by tests",
    }
}

vocabulary! {
    /// Package ecosystems through which a donor can enter a build.
    Ecosystem {
        Cargo = "CARGO": "a crate from a registry or git",
        Npm = "NPM": "a JavaScript/TypeScript package",
        Python = "PYTHON": "a Python distribution",
        Native = "NATIVE": "a native library linked via #[link] or build-script link directives",
    }
}

vocabulary! {
    /// The extinction gates. A donor is EXTINCT iff every gate passes.
    Gate {
        RuntimeEdges = "RUNTIME_EDGES_ZERO": "no runtime dependency edge to any donor package",
        BuildEdges = "BUILD_EDGES_ZERO": "no build dependency edge to any donor package",
        LinkedEdges = "LINKED_EDGES_ZERO": "no linked native artifact of the donor",
        TestEdges = "TEST_EDGES_ZERO": "no test/oracle execution of donor code; oracle outputs must be frozen fixtures",
        SourceImports = "SOURCE_IMPORTS_ZERO": "no source file imports the donor",
        ResidentSource = "RESIDENT_SOURCE_ZERO": "no donor source is tracked anywhere in the repository",
        CapabilityCoverage = "CAPABILITY_COVERAGE_FULL": "every required capability has a valid native replacement",
        ParityProofs = "PARITY_PROOFS_PASS": "every required capability has a fresh passing parity proof",
        RegressionTests = "REGRESSION_TESTS_PASS": "every required capability has a fresh passing regression proof",
        CanonicalReplacement = "CANONICAL_REPLACEMENT_EXISTS": "every replacement node exists, is active and sits in a canonical native role",
        TechnologyMapping = "TECHNOLOGY_MAPPING_FULL": "every required capability maps to a capability or technology of the canonical graph",
        Relevance = "CAPABILITY_RELEVANCE_RESOLVED": "every declared capability's production adoption is decided: relied on by ECDEV (and required) or NOT_ADOPTED with a tracked review (and not required)",
        KnowledgeResolved = "KNOWLEDGE_RESOLVED": "nothing is left to learn: no declared capability is unreviewed or an open study, benchmark or algorithm candidate (extinction removes a dependency only after its knowledge is absorbed)",
        CutoverDone = "CUTOVER_COMPLETED": "a cutover is declared",
        RollbackIndependent = "ROLLBACK_INDEPENDENT": "no node consumes, calls, controls or shims the donor",
    }
}

vocabulary! {
    /// Directory roles of the canonical repository shape.
    Role {
        Kernel = "KERNEL": "core/",
        Substrate = "SUBSTRATE": "substrate/",
        Domain = "DOMAIN": "domain/",
        Adapter = "ADAPTER": "adapter/",
        Application = "APPLICATION": "apps/",
        Tool = "TOOL": "tools/",
        Test = "TEST": "tests/",
        Research = "RESEARCH": "research/",
        Compat = "COMPAT": "compat/",
        Governance = "GOVERNANCE": ".ecdev/",
        Ci = "CI": ".github/",
        Toolchain = "TOOLCHAIN": ".cargo/",
        Agent = "AGENT": ".claude/",
    }
}

vocabulary! {
    /// Kinds of migration shim. Each must name an expiry condition.
    ShimKind {
        PathAlias = "PATH_ALIAS": "an old path kept resolvable after a move",
        ReExport = "RE_EXPORT": "a module re-exporting a moved node under its old name",
        DonorFallback = "DONOR_FALLBACK": "a switch that can route back to donor code",
    }
}

vocabulary! {
    /// Conditions under which a shim must be gone.
    ExpiryKind {
        WaveApplied = "WAVE_APPLIED": "the named migration wave has been applied",
        DonorExtinct = "DONOR_EXTINCT": "the named donor is effectively EXTINCT",
        NodeCanonical = "NODE_CANONICAL": "the named node sits at its canonical path",
    }
}

vocabulary! {
    /// Status of a migration wave.
    WaveStatus {
        Planned = "PLANNED": "not yet applied",
        Applied = "APPLIED": "applied: every node in the wave sits at its canonical path",
    }
}

/// Node kinds that own physical trees.
pub fn is_physical(kind: NodeKind) -> bool {
    !matches!(
        kind,
        NodeKind::Capability
            | NodeKind::Technology
            | NodeKind::Donor
            | NodeKind::External
            | NodeKind::Proof
    )
}

vocabulary! {
    /// Computed classification of two technologies implementing the same capability. Never
    /// declared.
    TechnologySharing {
        SharedImplementation = "SHARED_IMPLEMENTATION": "one canonical technology, materialized (REUSES) by other nodes",
        IndependentImplementation = "INDEPENDENT_IMPLEMENTATION": "a directly related family member (ALTERNATIVE_FOR, EVOLVES, ...) with different sources",
        DomainSpecialization = "DOMAIN_SPECIALIZATION": "one SPECIALIZES or GENERALIZES the other",
        SharedConcept = "SHARED_CONCEPT": "the same capability in one declared family, with no direct relation between the two",
        UnrelatedDuplicate = "UNRELATED_DUPLICATE": "the same capability with no declared relation (DUPLICATE_TECHNOLOGY)",
    }
}

/// Node kinds that may serve as a native replacement of a donor capability.
pub fn is_native_role(kind: NodeKind) -> bool {
    matches!(
        kind,
        NodeKind::Kernel
            | NodeKind::Substrate
            | NodeKind::Domain
            | NodeKind::Adapter
            | NodeKind::Application
            | NodeKind::Tool
            | NodeKind::Governance
    )
}

/// Edge kinds that carry a dependency scope.
pub fn is_dependency(kind: EdgeKind) -> bool {
    matches!(kind, EdgeKind::DependsOn | EdgeKind::Calls)
}

/// The identity namespace of every ECDEV-owned node.
pub const NAMESPACE: &str = "ecdev";

vocabulary! {
    /// Computed classification of a node's nativeness. Never declared.
    NativeStatus {
        Native = "NATIVE": "in-tree technology with no dependency on, import of, or link to any donor or external package",
        Dependent = "DEPENDENT": "native code that depends on or imports an external package that is not a donor",
        Wrapper = "WRAPPER": "native-looking code that depends on, imports, links or calls a donor; never a replacement",
        Compat = "COMPAT": "a migration shim; never a replacement",
        Abstract = "ABSTRACT": "a node without code (capability, donor, external, proof, planned)",
    }
}

vocabulary! {
    /// Kinds of compacted knowledge facts.
    FactKind {
        Statement = "STATEMENT": "a statement extracted from a document; identical statements are one fact",
        Definition = "DEFINITION": "a term and its definition; a newer definition supersedes an older one",
        Decision = "DECISION": "a recorded architectural decision and its status",
        LegacyClaim = "LEGACY_CLAIM": "a lifecycle claim imported from a legacy registry; never evidence",
        LegacyRecord = "LEGACY_RECORD": "a legacy registry field preserved for provenance",
        Document = "DOCUMENT": "the digest of a document whose knowledge was extracted",
        Milestone = "MILESTONE": "roadmap state of a milestone/<id> or gap/<id>, one key per field (scope, status, evidence, rank); a newer value supersedes an older one",
        Block = "BLOCK": "one block of an extracted document (heading, paragraph, list, code, table, ...) verbatim, keyed by its position; a document's blocks rebuild it byte for byte",
    }
}

/// Fact kinds an agent may assert directly (`ecdev-gov fact add`). DOCUMENT digests and BLOCK
/// outlines license the deletion of documents and legacy kinds license the expiry of legacy
/// shims, so those are written only by extraction and import.
pub fn is_assertable(kind: FactKind) -> bool {
    !matches!(
        kind,
        FactKind::Document | FactKind::LegacyClaim | FactKind::LegacyRecord | FactKind::Block
    )
}
