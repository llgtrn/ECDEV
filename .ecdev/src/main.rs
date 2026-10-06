//! The `ecdev-gov` command. The repository's own declarations are compiled in here so that rustc
//! type-checks them against `ecdev_governance::declare::decl`; at run time they are read by the same
//! reader every verifier uses, and the two readings must agree.

#[allow(dead_code)]
mod declared {
    use ecdev_governance::declare::decl::*;
    pub const REPOSITORY: Repository = include!("../declared/repository.rs");
    pub const DONORS: &[Donor] = include!("../declared/donors.rs");
    pub const MIGRATION: Migration = include!("../declared/migration.rs");
    pub const TECHNOLOGIES: &[Technology] = include!("../declared/technologies.rs");
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // The compiled declarations and the run-time reading of the same files must agree.
    let compiled = ecdev_governance::declare::decl::into_model(
        &declared::REPOSITORY,
        declared::DONORS,
        &declared::MIGRATION,
        declared::TECHNOLOGIES,
    );
    let own_root = ecdev_governance::default_root();
    if let Ok(read) = ecdev_governance::declare::load(&own_root) {
        if read != compiled {
            eprintln!("error: the run-time reading of .ecdev/declared disagrees with rustc's");
            std::process::exit(3);
        }
    }
    let (code, out) = ecdev_governance::cli::run(&args);
    print!("{out}");
    std::process::exit(code);
}
