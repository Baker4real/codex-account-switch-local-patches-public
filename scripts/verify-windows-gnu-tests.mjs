import { spawnSync } from 'node:child_process'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const manifestPath = resolve(repoRoot, 'src-tauri', 'Cargo.toml')
const expectedHost = 'x86_64-pc-windows-gnu'

function run(command, args, options = {}) {
  const result = spawnSync(command, args, {
    cwd: repoRoot,
    encoding: 'utf8',
    stdio: options.capture ? 'pipe' : 'inherit',
    shell: false,
  })

  if (result.error) {
    throw result.error
  }
  if (result.status !== 0) {
    process.exit(result.status ?? 1)
  }
  return result.stdout ?? ''
}

const rustcVersion = run('rustc', ['-Vv'], { capture: true })
const hostLine = rustcVersion
  .split(/\r?\n/)
  .find((line) => line.startsWith('host:'))
const host = hostLine?.replace('host:', '').trim()

if (host !== expectedHost) {
  console.error(
    `Expected Rust host ${expectedHost}, got ${host ?? 'unknown'}.\n` +
      'This verification targets the Windows GNU/MinGW linker path only.',
  )
  process.exit(1)
}

run('cargo', ['check', '--manifest-path', manifestPath, '--lib'])
run('cargo', [
  'test',
  '--manifest-path',
  manifestPath,
  'login_profile_also_writes_live_codex_home_when_target_is_active',
])
run('cargo', [
  'test',
  '--manifest-path',
  manifestPath,
  'login_profile_rejects_account_mismatch_without_overwriting_auth',
])
run('cargo', [
  'test',
  '--manifest-path',
  manifestPath,
  'switch_does_not_contaminate_stale_marker_profile_with_drifted_account',
])
run('cargo', [
  'test',
  '--manifest-path',
  manifestPath,
  'bootstrap_sync',
])
run('cargo', [
  'test',
  '--manifest-path',
  manifestPath,
  'login_browser_app_server_launch_routes_forced_edge_account_without_legacy_profile_dir',
])
run('cargo', [
  'test',
  '--manifest-path',
  manifestPath,
  'build_login_browser_command_uses_existing_browser_profile',
])
run('cargo', ['test', '--manifest-path', manifestPath, 'windows::profile_actions'])
