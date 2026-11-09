type Comments = (Option<&'static str>, Option<(&'static str, &'static str)>);

const SLASH: Comments = (Some("//"), Some(("/*", "*/")));
const SLASH_LINE: Comments = (Some("//"), None);
const BLOCK: Comments = (None, Some(("/*", "*/")));
const HASH: Comments = (Some("#"), None);
const HASH_BLOCK: Comments = (Some("#"), Some(("/*", "*/")));
const SEMI: Comments = (Some(";"), None);
const MARKUP: Comments = (None, Some(("<!--", "-->")));
const ML: Comments = (None, Some(("(*", "*)")));
const NONE: Comments = (None, None);

pub const LANGUAGES: &[(&str, &[&str], &[&str], Comments)] = &[
    ("astro", &[".astro"], &[], SLASH),
    ("bat", &[".bat", ".cmd"], &[], (Some("REM"), None)),
    ("c", &[".c", ".h"], &[], SLASH),
    ("clojure", &[".clj", ".cljs", ".cljc", ".edn"], &[], SEMI),
    ("cmake", &[".cmake"], &["cmakelists.txt"], HASH),
    ("common-lisp", &[".lisp", ".lsp"], &[], SEMI),
    ("cpp", &[".cpp", ".cc", ".cxx", ".hpp", ".hh", ".hxx"], &[], SLASH),
    ("csharp", &[".cs"], &[], SLASH),
    ("css", &[".css"], &[], BLOCK),
    ("d", &[".d"], &[], SLASH),
    ("dart", &[".dart"], &[], SLASH),
    ("docker", &[".dockerfile"], &["dockerfile", "containerfile"], HASH),
    ("dotenv", &[".env"], &[".env", ".env.local", ".env.example", ".dev.vars"], HASH),
    ("elixir", &[".ex", ".exs"], &[], HASH),
    ("elm", &[".elm"], &[], (Some("--"), Some(("{-", "-}")))),
    ("erlang", &[".erl", ".hrl"], &[], (Some("%"), None)),
    ("fish", &[".fish"], &[], HASH),
    ("fsharp", &[".fs", ".fsi", ".fsx"], &[], (Some("//"), Some(("(*", "*)")))),
    ("glsl", &[".glsl", ".vert", ".frag", ".geom", ".comp"], &[], SLASH),
    ("go", &[".go"], &[], SLASH),
    ("graphql", &[".graphql", ".gql"], &[], HASH),
    ("groovy", &[".groovy", ".gradle"], &["jenkinsfile"], SLASH),
    ("haskell", &[".hs", ".lhs"], &[], (Some("--"), Some(("{-", "-}")))),
    ("hcl", &[".hcl"], &[], HASH_BLOCK),
    ("hlsl", &[".hlsl", ".fx"], &[], SLASH),
    ("html", &[".html", ".htm", ".xhtml"], &[], MARKUP),
    ("ini", &[".ini", ".cfg", ".properties"], &[], (Some(";"), None)),
    ("java", &[".java"], &[], SLASH),
    ("javascript", &[".js", ".mjs", ".cjs"], &[], SLASH),
    ("json", &[".json", ".map"], &[], NONE),
    ("json5", &[".json5"], &[], SLASH),
    ("jsonc", &[".jsonc"], &["tsconfig.json", "jsconfig.json", ".eslintrc.json"], SLASH),
    ("jsx", &[".jsx"], &[], SLASH),
    ("julia", &[".jl"], &[], (Some("#"), Some(("#=", "=#")))),
    ("kotlin", &[".kt", ".kts"], &[], SLASH),
    ("latex", &[".tex", ".ltx"], &[], (Some("%"), None)),
    ("less", &[".less"], &[], SLASH),
    ("lua", &[".lua"], &[], (Some("--"), Some(("--[[", "]]")))),
    ("make", &[".mk"], &["makefile", "gnumakefile"], HASH),
    ("markdown", &[".md", ".markdown", ".mdown"], &[], MARKUP),
    ("mdx", &[".mdx"], &[], MARKUP),
    ("nginx", &[".nginx"], &["nginx.conf"], HASH),
    ("nim", &[".nim"], &[], (Some("#"), Some(("#[", "]#")))),
    ("nix", &[".nix"], &[], HASH_BLOCK),
    ("objective-c", &[".m"], &[], SLASH),
    ("objective-cpp", &[".mm"], &[], SLASH),
    ("ocaml", &[".ml", ".mli"], &[], ML),
    ("pascal", &[".pas", ".dpr"], &[], (Some("//"), Some(("{", "}")))),
    ("perl", &[".pl", ".pm"], &[], HASH),
    ("php", &[".php", ".phtml"], &[], SLASH),
    ("powershell", &[".ps1", ".psm1", ".psd1"], &[], (Some("#"), Some(("<#", "#>")))),
    ("prisma", &[".prisma"], &[], SLASH_LINE),
    ("proto", &[".proto"], &[], SLASH),
    ("python", &[".py", ".pyi", ".pyw"], &[], HASH),
    ("r", &[".r"], &[], HASH),
    ("racket", &[".rkt"], &[], SEMI),
    ("ruby", &[".rb", ".rake", ".gemspec"], &["gemfile", "rakefile"], HASH),
    ("rust", &[".rs"], &[], SLASH),
    ("sass", &[".sass"], &[], SLASH_LINE),
    ("scala", &[".scala", ".sc"], &[], SLASH),
    ("scheme", &[".scm", ".ss"], &[], SEMI),
    ("scss", &[".scss"], &[], SLASH),
    ("shellscript", &[".sh", ".bash", ".zsh", ".ksh"], &[".bashrc", ".zshrc", ".profile"], HASH),
    ("solidity", &[".sol"], &[], SLASH),
    ("sql", &[".sql"], &[], (Some("--"), Some(("/*", "*/")))),
    ("svelte", &[".svelte"], &[], MARKUP),
    ("swift", &[".swift"], &[], SLASH),
    ("terraform", &[".tf", ".tfvars"], &[], HASH_BLOCK),
    ("toml", &[".toml"], &["cargo.lock"], HASH),
    ("tsx", &[".tsx"], &[], SLASH),
    ("typescript", &[".ts", ".mts", ".cts"], &[], SLASH),
    ("vb", &[".vb"], &[], (Some("'"), None)),
    ("viml", &[".vim"], &[".vimrc"], (Some("\""), None)),
    ("vue", &[".vue"], &[], MARKUP),
    ("wgsl", &[".wgsl"], &[], SLASH),
    ("xml", &[".xml", ".xsd", ".xsl", ".xaml", ".csproj", ".svg"], &[], MARKUP),
    ("yaml", &[".yaml", ".yml"], &[], HASH),
    ("zig", &[".zig"], &[], SLASH_LINE),
];

pub const THEMES: &[&str] = &[
    "andromeeda", "aurora-x", "ayu-dark", "ayu-light", "ayu-mirage", "catppuccin-frappe",
    "catppuccin-latte", "catppuccin-macchiato", "catppuccin-mocha", "dark-plus", "dracula",
    "dracula-soft", "everforest-dark", "everforest-light", "github-dark", "github-dark-default",
    "github-dark-dimmed", "github-dark-high-contrast", "github-light", "github-light-default",
    "github-light-high-contrast", "gruvbox-dark-hard", "gruvbox-dark-medium", "gruvbox-dark-soft",
    "gruvbox-light-hard", "gruvbox-light-medium", "gruvbox-light-soft", "horizon", "horizon-bright",
    "houston", "kanagawa-dragon", "kanagawa-lotus", "kanagawa-wave", "laserwave", "light-plus",
    "material-theme", "material-theme-darker", "material-theme-lighter", "material-theme-ocean",
    "material-theme-palenight", "min-dark", "min-light", "monokai", "night-owl", "night-owl-light",
    "nord", "one-dark-pro", "one-light", "plastic", "poimandres", "red", "rose-pine", "rose-pine-dawn",
    "rose-pine-moon", "slack-dark", "slack-ochin", "snazzy-light", "solarized-dark", "solarized-light",
    "synthwave-84", "tokyo-night", "vesper", "vitesse-black", "vitesse-dark", "vitesse-light",
];

/// "one-dark-pro" -> "One Dark Pro".
pub fn theme_label(id: &str) -> String {
    match id {
        "dark-plus" => "Dark+".into(),
        "light-plus" => "Light+".into(),
        _ => id
            .split('-')
            .map(|word| {
                let mut letters = word.chars();
                letters.next().map_or_else(String::new, |first| {
                    first.to_uppercase().chain(letters).collect()
                })
            })
            .collect::<Vec<_>>()
            .join(" "),
    }
}

pub fn theme_key(label: &str) -> String {
    let key: String = label
        .chars()
        .filter_map(|c| match c {
            '+' => Some('p'),
            c if c.is_alphanumeric() => Some(c.to_ascii_lowercase()),
            _ => None,
        })
        .collect();
    key.strip_suffix("theme").map(str::to_owned).unwrap_or(key)
}
