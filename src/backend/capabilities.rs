bitflags::bitflags! {
    /// What a backend can do in a scope. Screens and key handlers read it to
    /// leave out what the host cannot do, instead of offering it and showing
    /// the error the call returns.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct Capabilities: u16 {
        const LIST_PIPELINES = 1;
        const LIST_RUNNERS = 1 << 1;
        /// Pausing, resuming and renaming runners.
        const MANAGE_RUNNERS = 1 << 2;
        const LIST_RELEASES = 1 << 3;
        const LIST_MILESTONES = 1 << 4;
        const LIST_BRANCHES = 1 << 5;
        /// Environments and their deployments.
        const LIST_ENVIRONMENTS = 1 << 6;
    }
}
