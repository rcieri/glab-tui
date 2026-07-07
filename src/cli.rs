use crate::app::App;

pub struct Cli {
    pub is_github: bool,
}

impl Cli {
    pub fn program(&self) -> &'static str {
        if self.is_github { "gh" } else { "glab" }
    }

    pub fn entity<'a>(&self, name: &'a str) -> &'a str {
        if self.is_github && name == "mr" {
            "pr"
        } else {
            name
        }
    }

    pub fn sub_update(&self) -> &'static str {
        if self.is_github { "edit" } else { "update" }
    }

    pub fn flag_description(&self) -> &'static str {
        if self.is_github {
            "--body"
        } else {
            "--description"
        }
    }

    pub fn flag_description_short(&self) -> &'static str {
        if self.is_github { "--body" } else { "-d" }
    }

    pub fn flag_branch(&self) -> &'static str {
        if self.is_github { "-r" } else { "-b" }
    }

    pub fn flag_input(&self) -> &'static str {
        if self.is_github { "-f" } else { "-i" }
    }

    pub fn flag_variable(&self) -> &'static str {
        if self.is_github { "-f" } else { "--variables" }
    }

    pub fn flag_web(&self) -> &'static str {
        if self.is_github { "--web" } else { "-w" }
    }

    pub fn input_separator(&self) -> &str {
        if self.is_github { "=" } else { ":" }
    }
}

pub struct UpdateCmd {
    pub is_github: bool,
    pub args: Vec<String>,
}

impl UpdateCmd {
    pub fn new(is_github: bool, entity: &str, iid: u64) -> Self {
        let e = if is_github && entity == "mr" {
            "pr"
        } else {
            entity
        };
        let cmd = if is_github { "edit" } else { "update" };
        Self {
            is_github,
            args: vec![e.to_string(), cmd.to_string(), iid.to_string()],
        }
    }

    pub fn flag(mut self, name: &str, value: &str) -> Self {
        let (name, value) = match (self.is_github, name) {
            (true, "-d" | "--description") => ("--body", value),
            (true, "--unlabel") if value == "all" => ("--label", ""),
            (true, "--unassign") => ("--assignee", ""),
            (true, "--target-branch") => ("--base", value),
            (true, "--milestone") if value == "0" => ("--milestone", ""),
            _ => (name, value),
        };
        self.args.push(name.to_string());
        self.args.push(value.to_string());
        self
    }

    pub fn flag_bool(mut self, name: &str) -> Self {
        self.args.push(name.to_string());
        self
    }

    pub fn build(&self) -> Vec<String> {
        self.args.clone()
    }
}

pub fn app_cli(app: &App) -> Cli {
    Cli {
        is_github: app
            .gitlab_client
            .as_ref()
            .map(|c| c.is_github)
            .unwrap_or(false),
    }
}
