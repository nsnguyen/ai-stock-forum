use ai_stock_forum::{
    agents, app, audit, config, domains, jobs, mcp, memory, persistence, policy,
    providers, recovery, rooms, runtime, runtimes, setup, skills, ui,
};

#[test]
fn phase_zero_exports_the_approved_module_boundaries() {
    let names = [
        agents::MODULE_NAME,
        app::MODULE_NAME,
        audit::MODULE_NAME,
        config::MODULE_NAME,
        domains::MODULE_NAME,
        jobs::MODULE_NAME,
        mcp::MODULE_NAME,
        memory::MODULE_NAME,
        persistence::MODULE_NAME,
        policy::MODULE_NAME,
        providers::MODULE_NAME,
        recovery::MODULE_NAME,
        rooms::MODULE_NAME,
        runtime::MODULE_NAME,
        runtimes::MODULE_NAME,
        setup::MODULE_NAME,
        skills::MODULE_NAME,
        ui::MODULE_NAME,
    ];
    assert_eq!(names.len(), 18);
}
