# TODO — Auth removal (WIP on `chore/remove-login`)

## Status
✅ Workspace `cargo check --workspace --tests` clean.
✅ `cargo build --release --workspace` clean (warnings only, 4m 29s).
✅ `pnpm install && pnpm -r build` — dashboard + autocomplete packages build.
✅ `precog install --no-confirm`, `precog setup --no-confirm`, `precog doctor` — no login gate, exit 0.
Pending: autocomplete interactive smoke (requires new shell session).

## Completed this session
- `fig_telemetry` — gutted CW client + `fig_auth::builder_id_token` tagging; dropped `amzn-codewhisperer-client` dep
- `fig_telemetry_core` — removed `From<SuggestionState> for amzn_codewhisperer_client::types::SuggestionState`; dropped `amzn-codewhisperer-client` dep
- `proto/fig.proto` — removed Auth*, Codewhisperer*, ListAvailableProfiles*, SetProfile*, UserLogout*, Profile messages
- `proto/local.proto` — removed Login/Logout commands
- `fig_desktop_api` — deleted requests/{auth,codewhisperer,profile}.rs; stripped handler match arms; removed `user_logged_in_callback`/`user_logout` trait methods
- `fig_ipc` — removed `login_command`/`logout_command`
- `figterm` — removed `inline` module + `on_prompt`; inline shell completion requests now warn-and-drop
- `q_cli`
  - deleted `cli/user.rs` and `tests/cli_user.rs`
  - removed `cli/internal/inline_shell_completion` + `InlineShellCompletion`/`InlineShellCompletionAccept` subcommands
  - stripped `fig_auth`/`fig_api_client` refs from `cli/{mod,installation,settings,uninstall,init,doctor/{mod,checks/{midway,sshd_config}},debug/mod}.rs` and `util/mod.rs`
  - dropped `amzn-codewhisperer-client` / `amzn-codewhisperer-streaming-client` workspace deps
- `fig_desktop` — deleted `auth_watcher`, `request/user.rs`; stripped `is_logged_in`/LOGIN_PATH/LOGIN_MENU_ID; simplified tray + dashboard onboarding gate
- `fig_settings`
  - deleted `migrations/005_auth_table.sql`; dropped from MIGRATIONS list
  - removed `get_auth_value`/`set_auth_value`/`unset_auth_value`/`is_auth_value_set` + `AUTH_TABLE_NAME` const + tests
  - dropped writes to `desktop.completedOnboarding` and read of `desktop.auth-watcher.logged-in`
- Dashboard (TS)
  - deleted `components/installs/modal/login/`, `hooks/store/useAuth.ts`, `api-bindings/src/{auth,codewhisperer,profile,user}.ts`
  - stripped `Auth`/`Codewhisperer`/`Profile`/`User` namespaces from `api-bindings/src/index.ts`
  - rewrote `pages/settings/preferences.tsx`, `pages/terminal/inline.tsx`, `lib/store.ts`, `App.tsx` to remove auth concepts
  - stripped broken send funcs + imports from `api-bindings/src/requests.ts`
- Deleted unused crates: `amzn-codewhisperer-client`, `amzn-codewhisperer-streaming-client`, `amzn-consolas-client`, `amzn-qdeveloper-streaming-client`; removed from workspace deps

## Verification still to run
- Autocomplete interactive smoke (open a fresh shell with the new binary on PATH; the `precog doctor` "terminal not running with latest integration" warning will clear on restart).
