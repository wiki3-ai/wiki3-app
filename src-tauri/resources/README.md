# Resources bundled into the Wiki3 .app

This directory is populated by \`build.rs\` with the pinned Deno binary
(\`deno-<target-triple>\`) before the Tauri bundler runs, so Deno ships
inside the installed app and the user never has to install one.
