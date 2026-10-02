//! The `ynventa` command. The repository's own declarations are compiled in here so that rustc
//! type-checks them against `ynventa::declare::decl`; at run time they are read by the same
//! reader every verifier uses, and the two readings must agree.

#[allow(dead_code)]
mod declared {
    use ynventa::declare::decl::*;
    pub const REPOSITORY: Repository = include!("../declared/repository.rs");
    pub const DONORS: &[Donor] = include!("../declared/donors.rs");
    pub const MIGRATION: Migration = include!("../declared/migration.rs");
    pub const TECHNOLOGIES: &[Technology] = include!("../declared/technologies.rs");
    pub const ORGANISM: Organism = include!("../declared/organism.rs");
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // The compiled declarations and the run-time reading of the same files must agree.
    let compiled = ynventa::declare::decl::into_model(
        &declared::REPOSITORY,
        declared::DONORS,
        &declared::MIGRATION,
        declared::TECHNOLOGIES,
        &declared::ORGANISM,
    );
    let own_root = ynventa::default_root();
    if let Ok(read) = ynventa::declare::load(&own_root) {
        if read != compiled {
            eprintln!("error: the run-time reading of .ynventa/declared disagrees with rustc's");
            std::process::exit(3);
        }
    }
    let (code, out) = ynventa::cli::run(&args);
    print!("{out}");
    std::process::exit(code);
}
