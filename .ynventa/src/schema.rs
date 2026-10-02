//! The closed vocabularies of Ynventa protocol v1. Every repository uses exactly these words;
//! nothing here may be extended per repository. The rendered form of these tables is part of the
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
        Repository = "REPOSITORY": "a physical shard of Chronica: one Git repository",
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
        Ynventa = "YNVENTA": "the Ynventa subsystem (.ynventa)",
        Capability = "CAPABILITY": "WHAT Chronica can do: an abstract behaviour, provided and required by nodes",
        Technology = "TECHNOLOGY": "HOW Chronica natively does it: a canonical native mechanism implementing capabilities",
        Donor = "DONOR": "an external OSS technology under the donor lifecycle",
        External = "EXTERNAL": "an observed external package that is not (yet) a registered donor",
        Proof = "PROOF": "an executable proof: a parity or regression test",
        System = "SYSTEM": "Chronica itself: the one logical system every shard belongs to",
        Organ = "ORGAN": "a conceptual organ of the Norl organism (abstract; declared only by the norl shard)",
        Material = "MATERIAL": "abstract developmental material a shard offers the organism: a world, observation, action, experience, curriculum or evaluation; its Concept says which",
    }
}

vocabulary! {
    /// The Chronica ontology: what a node means, whatever repository it lives in.
    Concept {
        System = "SYSTEM": "the whole of Chronica",
        Repository = "REPOSITORY": "a physical shard",
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
        World = "WORLD": "an environment the organism can be placed in",
        Action = "ACTION": "something the organism can do to a world",
        Experience = "EXPERIENCE": "a recorded episode of acting in a world",
        Curriculum = "CURRICULUM": "an ordered course of material that teaches",
        Evaluation = "EVALUATION": "a judged trial whose evidence decides growth",
        Organ = "ORGAN": "a part of the organism",
        Weight = "WEIGHT": "a learned parameter artifact",
    }
}

vocabulary! {
    /// Typed relations. One word per relation across Chronica; inverses (BELONGS_TO, depended-on-by,
    /// backlinks) are derived, never stored.
    EdgeKind {
        Contains = "CONTAINS": "structural containment",
        DependsOn = "DEPENDS_ON": "the source cannot build or run without the target (native build/run dependency, scoped)",
        Implements = "IMPLEMENTS": "the source technology or node implements the target capability or contract",
        Provides = "PROVIDES": "the source offers the target capability to Chronica",
        Consumes = "CONSUMES": "the source uses the target capability",
        Requires = "REQUIRES": "the source needs the target capability at run time, from whichever shard provides it",
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
        Generates = "GENERATES": "the source produces the target material (a world, observation, action, experience, curriculum or evaluation)",
        Feeds = "FEEDS": "developmental supply: the source material, technology or capability feeds the target Norl capability or organ",
        Teaches = "TEACHES": "the source curriculum teaches the target Norl capability or organ",
        Uses = "USES": "the source material uses the target material (an evaluation uses an experience or world; an experience happens in a world), or the source repository uses or studies the target donor (one edge per declared donor; the same upstream is one donor node however many repositories use it)",
        Observes = "OBSERVES": "the source perceives the target world or observation",
        ActsOn = "ACTS_ON": "the source acts on the target world",
        EvaluatedBy = "EVALUATED_BY": "the source capability or organ is judged by the target evaluation",
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
        Canonical = "CANONICAL": "proven, declared canonical, and no unrelated duplicate in the linked system",
        Adopted = "ADOPTED": "reused natively by at least one other shard of the linked system",
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
    /// THE donor lifecycle. One ladder for the whole ecosystem; effective states are computed
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
        NorlRelevanceResolved = "NORL_RELEVANCE_RESOLVED": "every required capability feeds a Norl capability or is declared not relevant to Norl",
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
    /// Exceptional donor states and their exact semantics.
    ExceptionKind {
        Blocked = "BLOCKED": "progress halted by a named blocker; the donor keeps its effective state and stays in every denominator",
        Rejected = "REJECTED": "evaluated and never adopted; legal only with zero observed edges now and in all census history",
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
        NorlRelevance = "NORL_RELEVANCE_RESOLVED": "every required capability's relevance to Norl is resolved",
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
        Ynventa = "YNVENTA": ".ynventa/",
        Ci = "CI": ".github/",
        Toolchain = "TOOLCHAIN": ".cargo/",
        Agent = "AGENT": ".claude/",
        Legacy = "LEGACY": ".atlas/ (legacy input; legal only while a LEGACY_INPUT shim is active)",
    }
}

vocabulary! {
    /// Kinds of migration shim. Each must name an expiry condition.
    ShimKind {
        LegacyInput = "LEGACY_INPUT": "a legacy knowledge tree read by the importer",
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
        LegacyImported = "LEGACY_IMPORTED": "every legacy donor record is imported and every legacy document is compacted",
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
            | NodeKind::System
            | NodeKind::Organ
            | NodeKind::Material
    )
}

vocabulary! {
    /// THE growth ladder of a Norl organism capability. Effective states are computed from the
    /// linked graph and evaluation evidence; declared states are only claims.
    GrowthState {
        Defined = "DEFINED": "the capability and its organ exist in the norl graph",
        Exposed = "EXPOSED": "a norl physical node provides or implements it",
        Experienced = "EXPERIENCED": "at least one experience, generated by a node of its shard, feeds it",
        Evaluated = "EVALUATED": "evaluated by at least one evaluation with fresh passing evidence",
        Learned = "LEARNED": "evaluated, and its backend is a native learned weight (tiny or scaled)",
        Native = "NATIVE": "evaluated, its backend is native, and no borrowed backend or donor is in its provision path",
    }
}

vocabulary! {
    /// How a capability proven in a source repository is promoted into Norl: what, if any, of the
    /// source's implementation crosses. Code crosses only as a materialized technology (REUSES)
    /// or with its full source provenance; Norl never path-depends on a sibling repository.
    PromotionImplementation {
        SharedImplementation = "SHARED_IMPLEMENTATION": "Norl compiles the source technology in natively (its destination node REUSES it): one canonical implementation",
        IndependentImplementation = "INDEPENDENT_IMPLEMENTATION": "Norl implements the capability itself from the source's semantics; no source code crosses",
        CopiedAndDiverged = "COPIED_AND_DIVERGED": "source code was copied into Norl and diverges; it carries its source provenance (repository, NodeId, commit, technology)",
        AdaptedImplementation = "ADAPTED_IMPLEMENTATION": "source code was adapted to Norl's model; it carries its source provenance (repository, NodeId, commit, technology)",
        ReferenceOnly = "REFERENCE_ONLY": "the source is studied as a reference; Norl implements nothing from it under this promotion",
    }
}

vocabulary! {
    /// THE promotion ladder: a source repository's proven capability becoming Norl's. Effective
    /// states are computed; the source's proof is the first rung and never raises Norl's growth
    /// (SOURCE_REPO_CAN_DO(X) != NORL_CAN_DO(X)).
    PromotionState {
        ProvenInSource = "PROVEN_IN_SOURCE": "the source repository proves it with fresh passing evidence at its node, capability or technology (decided at link)",
        NorlRelevanceResolved = "NORL_RELEVANCE_RESOLVED": "it names a defined organism capability of Norl and states why Norl needs it",
        ImportCandidate = "IMPORT_CANDIDATE": "its source provenance is complete: repository, node, commit, and a capability or technology; its relation is FEEDS or TEACHES",
        PromotionDesigned = "PROMOTION_DESIGNED": "its transformation into Norl is stated",
        NativeNorlImplementation = "NATIVE_NORL_IMPLEMENTATION": "an active Norl physical destination node exists (and REUSES the source technology for a shared implementation); never for REFERENCE_ONLY",
        NorlTested = "NORL_TESTED": "a Norl-owned proof in the destination node passes, fresh",
        NorlEvaluated = "NORL_EVALUATED": "the destination capability's Norl growth is EVALUATED or beyond (a norl-owned evaluation passes)",
        NorlNative = "NORL_NATIVE": "the destination capability's Norl growth is NATIVE",
    }
}

vocabulary! {
    /// What other repositories have proven of what feeds an organism capability: reported beside
    /// its growth (Norl's maturity), never part of it.
    SourceMaturity {
        Unfed = "NONE": "nothing of another repository feeds or is promoted into it",
        Fed = "FED": "material, a technology or a promotion of another repository feeds it, without fresh passing source evidence",
        ProvenInSource = "PROVEN_IN_SOURCE": "a source repository proves what feeds or is promoted into it with fresh passing evidence",
    }
}

vocabulary! {
    /// Where an organism capability's cognition comes from.
    BackendKind {
        Deterministic = "DETERMINISTIC": "native rule or stub cognition; no learned weights",
        BorrowedWeight = "BORROWED_WEIGHT": "an external model through an adapter",
        NativeTinyWeight = "NATIVE_TINY_WEIGHT": "a native learned weight of small scale",
        NativeScaledWeight = "NATIVE_SCALED_WEIGHT": "a native learned weight at scale",
    }
}

vocabulary! {
    /// Computed classification of two technologies implementing the same capability. Never
    /// declared.
    TechnologySharing {
        SharedImplementation = "SHARED_IMPLEMENTATION": "one canonical technology, materialized (REUSES) by other shards",
        IndependentImplementation = "INDEPENDENT_IMPLEMENTATION": "a directly related family member (ALTERNATIVE_FOR, EVOLVES, ...) with different sources",
        DomainSpecialization = "DOMAIN_SPECIALIZATION": "one SPECIALIZES or GENERALIZES the other",
        SharedConcept = "SHARED_CONCEPT": "the same capability in one declared family, with no direct relation between the two",
        UnrelatedDuplicate = "UNRELATED_DUPLICATE": "the same capability with no declared relation (DUPLICATE_TECHNOLOGY)",
    }
}

/// The shard that is the organism. Organs and organism capabilities are legal only there.
pub const NORL_SHARD: &str = "norl";

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
            | NodeKind::Ynventa
    )
}

/// Edge kinds that carry a dependency scope.
pub fn is_dependency(kind: EdgeKind) -> bool {
    matches!(kind, EdgeKind::DependsOn | EdgeKind::Calls)
}

/// The one system every shard belongs to. Every node identity is derived under it.
pub const SYSTEM: &str = "chronica";

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

/// Fact kinds an agent may assert directly (`ynventa fact add`). DOCUMENT digests and BLOCK
/// outlines license the deletion of documents and legacy kinds license the expiry of legacy
/// shims, so those are written only by extraction and import.
pub fn is_assertable(kind: FactKind) -> bool {
    !matches!(
        kind,
        FactKind::Document | FactKind::LegacyClaim | FactKind::LegacyRecord | FactKind::Block
    )
}
