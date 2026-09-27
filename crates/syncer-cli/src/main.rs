mod config;
use anyhow::{Context as _, Result, bail, ensure};
use clap::{Parser, Subcommand};
use config::{Config, Endpoint, Reporter, Source};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};
use syncer_core::{Context, Extensions, Layer, extension::Installed, storage};

#[derive(Parser)]
#[command(
    name = "syncer",
    version,
    about = "Layered, patch-first file policy synchronization"
)]
struct Cli {
    /// Local enrollment/cache/extension directory.
    #[arg(long, global = true)]
    state_dir: Option<PathBuf>,
    /// Project root used to resolve relative targets.
    #[arg(long, global = true)]
    project: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    /// Enroll a local or extension-backed policy source. Higher priorities win.
    Add {
        /// Enroll raw file content instead of an HCL policy.
        #[arg(long)]
        asset: bool,
        name: String,
        uri: String,
        #[arg(long)]
        priority: Option<i32>,
        #[arg(long = "allow-root")]
        roots: Vec<PathBuf>,
        #[arg(long)]
        extension: Option<String>,
        #[arg(long = "credential")]
        credentials: Vec<String>,
        #[arg(long)]
        sha256: Option<String>,
    },
    /// List enrolled sources and their local trust settings.
    List,
    /// Remove an enrolled policy source (does not undo previous changes).
    Remove { name: String },
    /// Fetch and validate policy sources without modifying targets.
    Fetch,
    /// Plan all policies, then apply if every rule and inherited constraint passes.
    Apply {
        #[arg(long)]
        dry_run: bool,
        #[arg(long)]
        diff: bool,
        #[arg(long)]
        offline: bool,
        #[arg(long = "role")]
        roles: Vec<String>,
        /// Exit 2 when drift is found (implies dry-run).
        #[arg(long)]
        check: bool,
    },
    /// Repeatedly refresh and apply policies; suitable for OS service managers.
    Daemon {
        #[arg(long,default_value_t=86400,value_parser=clap::value_parser!(u64).range(1..))]
        interval: u64,
        #[arg(long)]
        once: bool,
        #[arg(long = "role")]
        roles: Vec<String>,
    },
    /// Validate the HCL policy syntax and schema.
    Validate { file: PathBuf },
    /// Publish a local policy to an enrolled source (e.g. personal rules to Drive).
    Push {
        name: String,
        file: PathBuf,
        #[arg(long)]
        dry_run: bool,
    },
    /// Manage explicitly trusted native extensions.
    Extension {
        #[command(subcommand)]
        command: ExtensionCommand,
    },
    /// Opt-in encrypted compliance reporting with locally pinned recipient keys.
    Report {
        #[command(subcommand)]
        command: ReportCommand,
    },
}
#[derive(Subcommand)]
enum ExtensionCommand {
    Install {
        path: PathBuf,
        #[arg(long)]
        sha256: Option<String>,
    },
    List,
    Remove {
        name: String,
    },
}
#[derive(Subcommand)]
enum ReportCommand {
    /// Generate a provider age keypair. The private key never enters enrollment.
    Keygen {
        identity: PathBuf,
    },
    /// Summarize locally downloaded encrypted reports with the provider private key.
    Summarize {
        directory: PathBuf,
        #[arg(long)]
        identity: PathBuf,
    },
    Enable {
        policy: String,
        sink: String,
        #[arg(long)]
        recipient: String,
        #[arg(long)]
        extension: Option<String>,
        #[arg(long = "credential")]
        credentials: Vec<String>,
    },
    Disable {
        policy: String,
    },
    /// Retry sending encrypted reports retained after transport failure.
    Flush,
    List,
}
#[tokio::main]
async fn main() {
    match run(Cli::parse()).await {
        Ok(code) => std::process::exit(code),
        Err(e) => {
            eprintln!("syncer: {e:#}");
            std::process::exit(1);
        }
    }
}
fn absolute(path: &Path, project: &Path) -> Result<PathBuf> {
    storage::normalize(&if path.is_absolute() {
        path.into()
    } else {
        project.join(path)
    })
}
async fn run(cli: Cli) -> Result<i32> {
    let cwd = std::env::current_dir()?.canonicalize()?;
    let project = cli
        .project
        .as_deref()
        .map(|p| absolute(p, &cwd))
        .transpose()?
        .unwrap_or(cwd);
    let project = project
        .canonicalize()
        .context("project directory must exist")?;
    let home = dirs::home_dir()
        .context("cannot determine home directory")?
        .canonicalize()?;
    let state = absolute(
        &cli.state_dir.unwrap_or_else(|| {
            dirs::config_dir()
                .unwrap_or_else(|| home.join(".config"))
                .join("syncer")
        }),
        &project,
    )?;
    let context = Context { home, project };
    let mut config = Config::read(&state)?;
    // Read-only commands and dry-runs must not create a lock file or state directory.
    let read_only = matches!(
        &cli.command,
        Command::List
            | Command::Validate { .. }
            | Command::Apply { dry_run: true, .. }
            | Command::Apply { check: true, .. }
            | Command::Push { dry_run: true, .. }
            | Command::Extension {
                command: ExtensionCommand::List
            }
            | Command::Report {
                command: ReportCommand::List | ReportCommand::Summarize { .. }
            }
    );
    let daemon = matches!(&cli.command, Command::Daemon { .. });
    let _lock = if read_only || daemon {
        None
    } else {
        Some(storage::Lock::acquire(&state)?)
    };
    if !read_only {
        config = Config::read(&state)?;
    }
    match cli.command {
        Command::Validate { file } => {
            let data = std::fs::read_to_string(file)?;
            let p = syncer_language::parse(&data)?;
            println!(
                "valid: {} ({} base rules, {} roles)",
                p.name,
                p.rules.len(),
                p.roles.len()
            );
        }
        Command::List => println!("{}", serde_json::to_string_pretty(&config.sources)?),
        Command::Remove { name } => {
            let n = config.sources.len();
            config.sources.retain(|s| s.name != name);
            ensure!(n != config.sources.len(), "source {name} is not enrolled");
            config.save(&state)?;
        }
        Command::Add {
            asset,
            name,
            uri,
            priority,
            roots,
            extension,
            credentials,
            sha256,
        } => {
            ensure!(syncer_language::valid_name(&name), "invalid source name");
            ensure!(
                !config.sources.iter().any(|s| s.name == name),
                "source {name} already exists; remove it before re-enrollment"
            );
            let priority = priority.unwrap_or(
                config
                    .sources
                    .iter()
                    .map(|s| s.priority)
                    .max()
                    .unwrap_or(-10)
                    .checked_add(10)
                    .context("priority overflow")?,
            );
            ensure!(
                !config.sources.iter().any(|s| s.priority == priority),
                "source priorities must be unique"
            );
            if let Some(pin) = &sha256 {
                ensure!(
                    pin.len() == 64
                        && pin
                            .bytes()
                            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()),
                    "SHA-256 must be 64 lowercase hex digits"
                );
            }
            let roots = if roots.is_empty() {
                vec![context.project.clone()]
            } else {
                roots
                    .iter()
                    .map(|r| {
                        absolute(r, &context.project).and_then(|p| {
                            storage::reject_symlinks(&p)?;
                            ensure!(p.is_dir(), "approved root must already be a directory");
                            Ok(p)
                        })
                    })
                    .collect::<Result<_>>()?
            };
            let source = Source {
                asset,
                name,
                endpoint: Endpoint::new(&uri, extension, &credentials, &context.project)?,
                priority,
                roots,
                sha256,
            };
            let extensions = Extensions::load(&config.extensions).await?;
            let cache = config::fetch(&source, &extensions, &state, false).await?;
            config.sources.push(source.clone());
            config.sources.sort_by_key(|s| s.priority);
            config.save(&state)?;
            config::save_cache(&source, &cache, &state)?;
            println!("enrolled {} (priority {})", source.name, source.priority);
        }
        Command::Fetch => {
            let extensions = Extensions::load(&config.extensions).await?;
            let mut fetched = vec![];
            for source in &config.sources {
                fetched.push((
                    source,
                    config::fetch(source, &extensions, &state, false).await?,
                ));
            }
            for (source, cache) in fetched {
                config::save_cache(source, &cache, &state)?;
                println!("{} {}", source.name, cache.digest);
            }
        }
        Command::Apply {
            dry_run,
            diff,
            offline,
            roles,
            check,
        } => {
            return apply(
                &config,
                &state,
                &context,
                ApplyOptions {
                    dry_run: dry_run || check,
                    diff,
                    offline,
                    roles: &roles,
                    check,
                },
            )
            .await;
        }
        Command::Daemon {
            interval,
            once,
            roles,
        } => loop {
            let cycle_lock = storage::Lock::acquire(&state)?;
            let latest = Config::read(&state)?;
            match apply(
                &latest,
                &state,
                &context,
                ApplyOptions {
                    dry_run: false,
                    diff: false,
                    offline: false,
                    roles: &roles,
                    check: false,
                },
            )
            .await
            {
                Ok(code) => {
                    if once {
                        return Ok(code);
                    }
                }
                Err(e) => {
                    if once {
                        return Err(e);
                    }
                    eprintln!("syncer daemon: {e:#}; retrying in {interval}s");
                }
            }
            drop(cycle_lock);
            tokio::select! {_=tokio::time::sleep(Duration::from_secs(interval))=>{},_=tokio::signal::ctrl_c()=>break}
        },
        Command::Push {
            name,
            file,
            dry_run,
        } => {
            let source = config
                .sources
                .iter()
                .find(|s| s.name == name)
                .context("unknown source")?;
            let data = std::fs::read(file)?;
            if !source.asset {
                syncer_language::parse(std::str::from_utf8(&data)?)?;
            }
            if let Some(pin) = &source.sha256 {
                ensure!(
                    syncer_core::digest(&data) == *pin,
                    "published policy would differ from pinned digest; re-enroll source first"
                );
            }
            if dry_run {
                println!("would publish {} bytes to {}", data.len(), source.name);
            } else {
                let extensions = Extensions::load(&config.extensions).await?;
                let cache = config::fetch(source, &extensions, &state, true)
                    .await
                    .context("push requires a previously fetched revision")?;
                ensure!(
                    cache.revision.is_some(),
                    "source does not provide a revision for conditional publish"
                );
                source
                    .endpoint
                    .call(
                        &extensions,
                        "publish",
                        Some(&data),
                        cache.revision.as_deref(),
                    )
                    .await?;
                let fresh = config::fetch(source, &extensions, &state, false).await?;
                config::save_cache(source, &fresh, &state)?;
                println!("published {}", source.name);
            }
        }
        Command::Extension { command } => match command {
            ExtensionCommand::List => {
                let extensions = Extensions::load(&config.extensions).await?;
                println!("{}", serde_json::to_string_pretty(&extensions.manifests())?);
            }
            ExtensionCommand::Install { path, sha256 } => {
                let path = absolute(&path, &context.project)?;
                storage::reject_symlinks(&path)?;
                let bytes = std::fs::read(&path)?;
                let digest = syncer_core::digest(&bytes);
                if let Some(pin) = sha256 {
                    ensure!(digest == pin, "extension SHA-256 mismatch");
                }
                let destination = state.join("extensions").join(format!(
                    "{}-{}",
                    digest,
                    path.file_name()
                        .context("invalid library path")?
                        .to_string_lossy()
                ));
                storage::atomic_write(&destination, &bytes, None)?;
                let installed = Installed {
                    path: destination,
                    sha256: digest,
                };
                let mut all = config.extensions.clone();
                all.push(installed);
                let extensions = Extensions::load(&all).await?;
                config.extensions = all;
                config.save(&state)?;
                println!(
                    "installed; {} extensions enabled",
                    extensions.manifests().len()
                );
            }
            ExtensionCommand::Remove { name } => {
                let mut keep = vec![];
                let mut found = false;
                for install in &config.extensions {
                    let loaded = Extensions::load(std::slice::from_ref(install)).await?;
                    if loaded.manifests()[0].name == name {
                        found = true;
                    } else {
                        keep.push(install.clone());
                    }
                }
                ensure!(found, "extension {name} is not installed");
                config.extensions = keep;
                config.save(&state)?;
            }
        },
        Command::Report { command } => match command {
            ReportCommand::Keygen { identity } => {
                let path = absolute(&identity, &context.project)?;
                ensure!(
                    !path.exists(),
                    "refusing to overwrite an existing private key"
                );
                let (secret, recipient) = syncer_core::report::keygen();
                storage::atomic_write(&path, format!("{secret}\n").as_bytes(), None)?;
                println!("{recipient}");
            }
            ReportCommand::Summarize {
                directory,
                identity,
            } => {
                let identity = std::fs::read_to_string(identity)?;
                let mut encrypted = vec![];
                for entry in std::fs::read_dir(directory)? {
                    let path = entry?.path();
                    if path.extension().is_some_and(|e| e == "age") {
                        encrypted
                            .push(storage::read_optional(&path)?.context("report disappeared")?);
                    }
                }
                println!(
                    "{}",
                    serde_json::to_string_pretty(&syncer_core::report::summarize(
                        &identity, &encrypted
                    )?)?
                );
            }
            ReportCommand::List => println!("{}", serde_json::to_string_pretty(&config.reports)?),
            ReportCommand::Enable {
                policy,
                sink,
                recipient,
                extension,
                credentials,
            } => {
                ensure!(syncer_language::valid_name(&policy), "invalid policy name");
                let secret = config
                    .installation_secret
                    .get_or_insert_with(|| uuid::Uuid::new_v4().to_string());
                syncer_core::report::encrypt(&recipient, secret, &policy, &[])?;
                ensure!(
                    !config.reports.iter().any(|r| r.policy == policy),
                    "reporting already enrolled for this policy; disable before changing recipient"
                );
                config.reports.push(Reporter {
                    policy,
                    endpoint: Endpoint::new(&sink, extension, &credentials, &context.project)?,
                    recipient,
                });
                config.save(&state)?;
                println!("encrypted reporting enabled");
            }
            ReportCommand::Disable { policy } => {
                config.reports.retain(|r| r.policy != policy);
                config.save(&state)?;
            }
            ReportCommand::Flush => {
                let extensions = Extensions::load(&config.extensions).await?;
                flush(&config, &state, &extensions).await?;
            }
        },
    }
    Ok(0)
}
struct ApplyOptions<'a> {
    dry_run: bool,
    diff: bool,
    offline: bool,
    roles: &'a [String],
    check: bool,
}
async fn apply(
    config: &Config,
    state: &Path,
    context: &Context,
    options: ApplyOptions<'_>,
) -> Result<i32> {
    ensure!(
        !config.sources.is_empty(),
        "no policy sources; use syncer add NAME PATH_OR_URL"
    );
    let extensions = Extensions::load(&config.extensions).await?;
    let mut layers = vec![];
    let mut assets = std::collections::BTreeMap::new();
    let mut caches = vec![];
    let mut sources: Vec<_> = config.sources.iter().collect();
    sources.sort_by_key(|s| s.priority);
    ensure!(
        sources.windows(2).all(|s| s[0].priority != s[1].priority),
        "source priorities must be unique"
    );
    for source in sources {
        let cache = config::fetch(source, &extensions, state, options.offline).await?;
        if source.asset {
            assets.insert(source.name.clone(), config::cache_bytes(source, &cache)?);
            caches.push((source, cache));
            continue;
        }
        let policy = syncer_language::parse(&cache.data)?;
        layers.push(Layer {
            policy,
            roots: source.roots.clone(),
            revision: cache.digest.clone(),
        });
        caches.push((source, cache));
    }
    let resolved = syncer_core::resolve(&layers, options.roles)?;
    let mut plan =
        syncer_core::engine::plan_with_assets(&resolved, context, &extensions, &assets).await?;
    // Policies cannot rewrite their own enrollment, extension registry or protected state.
    for file in &plan.files {
        for source in &config.sources {
            if let Some(path) = source.endpoint.local_path()? {
                ensure!(
                    !storage::same_path(&file.path, &path),
                    "target is an active source file ({}); synchronize a staging copy and publish explicitly with syncer push",
                    source.name
                );
            }
        }
        ensure!(
            !storage::is_within(&file.path, state),
            "policies cannot write inside syncer's state directory"
        );
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&plan.summary(options.diff))?
    );
    if options.dry_run {
        return Ok(if !plan.compliant() {
            3
        } else if options.check && plan.changed() > 0 {
            2
        } else {
            0
        });
    }
    let compliant = plan.compliant();
    if compliant {
        if let Some(backup) = plan.apply(state)? {
            eprintln!("backups: {}", backup.display());
        }
        for (source, cache) in caches {
            config::save_cache(source, &cache, state)?;
        }
    } else {
        for status in &mut plan.status {
            status.after = status.before;
        }
    }
    for reporter in &config.reports {
        if !plan.status.iter().any(|s| s.policy == reporter.policy) {
            continue;
        }
        let secret = config
            .installation_secret
            .as_ref()
            .context("missing report installation secret")?;
        let encrypted = syncer_core::report::encrypt(
            &reporter.recipient,
            secret,
            &reporter.policy,
            &plan.status,
        )?;
        let directory = state.join("outbox").join(&reporter.policy);
        storage::private_dir(&directory)?;
        let file = directory.join(format!("{}.age", uuid::Uuid::new_v4()));
        storage::atomic_write(&file, &encrypted, None)?;
    }
    if let Err(e) = flush(config, state, &extensions).await {
        eprintln!("encrypted reports retained for retry: {e:#}");
    }
    if !compliant {
        bail!("unresolved rule violations; no target files changed");
    }
    Ok(0)
}
async fn flush(config: &Config, state: &Path, extensions: &Extensions) -> Result<()> {
    for reporter in &config.reports {
        let directory = state.join("outbox").join(&reporter.policy);
        if !directory.exists() {
            continue;
        }
        storage::reject_symlinks(&directory)?;
        for entry in std::fs::read_dir(directory)? {
            let path = entry?.path();
            if path.extension().is_none_or(|e| e != "age") {
                continue;
            }
            let data = storage::read_optional(&path)?.context("missing encrypted report")?;
            ensure!(
                data.starts_with(b"age-encryption.org/v1\n"),
                "outbox file is not age ciphertext"
            );
            reporter
                .endpoint
                .call(extensions, "report", Some(&data), None)
                .await?;
            std::fs::remove_file(path)?;
        }
    }
    Ok(())
}
