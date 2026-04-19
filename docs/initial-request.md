This is the request given to GitHub Copilot agent for initiating the Wiki3 App project:
https://github.com/wiki3-ai/wiki3-app/tasks/722abd3e-69a6-474a-ad3c-b5051a35d0d3?author=jimwhite
https://github.com/wiki3-ai/wiki3-app/pull/2

---

Implement the next milestone for wiki3.ai’s current **GitHub-based publishing flow**, while keeping the design portable enough to later swap in Cloudflare/R2. The app repo is **wiki3-app**, a macOS-first Tauri 2 app that loads wiki3.ai, preserves JupyterLite state, and already supports a local dev URL override via `WIKI3_DEV_URL`. ([GitHub][1]) The current site repo is **wiki3-ai-site**, which is the source for `wiki3.ai` and is heavily notebook/JupyterLite-oriented. ([GitHub][2]) The template repo is **wiki3-ai-template**, which is an incomplete but relevant JupyterLite/static-site template with `deploy.sh`, `rebuild.sh`, `.github`, and demo content. ([GitHub][3])

Build the first end-to-end publishing workflow with these user-visible capabilities:

1. **Create repo from template**
2. **Fork existing repo**
3. **Push local changes to repo**
4. **Publish/update site**

Use GitHub as the current backend, but structure the code so GitHub is an adapter, not the core domain model. GitHub supports creating repositories from templates and creating forks via REST APIs, and GitHub Pages publishes static files pushed to a repository. ([GitHub Docs][4])

## Product and architecture requirements

Treat the app as a **local-first desktop tool**. The user has a local working copy of a site. The app should help them create or connect that local repo, edit locally, then push and publish intentionally. Do **not** build any shared real-time service. Do **not** add Cloudflare code in this step.

Implement a thin abstraction layer with interfaces like:

* `RepoProvider`
* `PublishProvider`
* `WorkspaceManager`

Then provide a first concrete implementation:

* `GitHubRepoProvider`
* `GitHubPagesPublishProvider` or a combined `GitHubProvider` if simpler for this milestone

The abstraction should make it straightforward later to add:

* bare Git remotes
* Codeberg
* Cloudflare Artifacts
* R2/static publish targets

But for now, only GitHub must work.

## Required flows

### A. Create repo from template

Given a template repo, initially defaulting to `wiki3-ai/wiki3-ai-template`, create a new repository for the authenticated user or selected org using GitHub’s template-repository flow. GitHub supports repository creation from a template repo. ([GitHub Docs][4])

Then:

* clone the new repo locally into an app-managed workspace directory
* record workspace metadata locally
* detect/configure the publish source expected by the current repo layout
* open the workspace in the app’s site/editor flow

Minimum fields:

* owner
* repo name
* visibility
* description
* template owner/repo
* default publish settings

### B. Fork existing repo

Given a GitHub repo URL, fork it for the authenticated user using GitHub’s fork API. GitHub notes that fork creation is asynchronous, so poll until the new repo is available before cloning/opening it. ([GitHub Docs][5])

Then:

* clone fork locally
* add `upstream` remote pointing to the source repo
* add `origin` remote pointing to the fork
* record workspace metadata locally

### C. Push local changes

Support pushing local file changes from the app-managed workspace to GitHub.

Requirements:

* authentication should use secure GitHub auth appropriate for desktop use
* do not hardcode tokens into remote URLs in persisted config
* store credentials securely using platform facilities if available
* support basic status display: dirty files, current branch, last commit, push result
* if there are no commits yet, initialize repo state as needed
* support commit message entry
* support pushing current branch to origin

GitHub documents PATs for API/CLI authentication, but the implementation should prefer a safer desktop auth path if practical; if a PAT is used in this milestone, isolate that behind a credential service and keep the transport swappable. ([GitHub Docs][6])

### D. Publish/update site

Automate the current GitHub Pages-style publish flow from the workspace.

GitHub Pages publishes static files pushed to the configured publishing source, and supports publishing from a branch or `/docs` folder. ([GitHub Docs][7])

Implement a publish command that:

* builds or prepares the static site from the workspace
* determines the current publishing strategy for the repo
* pushes the generated output to the configured publishing source
* reports the resulting site URL
* supports repeat publish/update

For this milestone, support these two modes in code, even if only one is enabled by default:

* **docs-folder mode** on `main`
* **gh-pages branch mode**

Auto-detect existing repo convention where possible. If the repo already has one of these conventions, preserve it. If not, prefer the simplest convention compatible with the current template and repo layout.

## Repo-specific implementation guidance

Use the existing repos as inputs to the design:

* `wiki3-ai-site` is the current source repo for wiki3.ai and includes Jupyter/JupyterLite-related folders like `docs`, `files`, `jupyterlab-publish`, `jupyterlite_wiki_addon`, `pages`, plus config files like `jupyter-lite.json` and `jupyter_lite_config.json`. ([GitHub][2])
* `wiki3-ai-template` includes `.github`, `deploy.sh`, `rebuild.sh`, `jupyter-lite.json`, `files`, `jupyterlite_demo`, `packages`, and `repl`, and its README describes it as a modernized JupyterLite demo/template. ([GitHub][3])
* `wiki3-app` is a Tauri 2 desktop app with Rust backend and TypeScript frontend modules, with trusted-origin logic, desktop host commands, permission gating, and dev URL override. ([GitHub][1])

Use that to decide where to put new code. Favor:

* Rust/Tauri backend for filesystem, git, credential storage, OS integration
* TypeScript frontend for workflow UI and status display

## Concrete implementation tasks

1. Add a **workspace model**

   * local path
   * provider type (`github`)
   * owner/repo
   * branch
   * remotes
   * publish mode
   * site URL
   * template/fork lineage metadata

2. Add a **GitHub auth module**

   * secure token storage
   * authenticated API client
   * error handling for expired or insufficient scopes

3. Add a **repo operations module**

   * create from template
   * fork repo
   * clone repo
   * add/remove/list remotes
   * get status
   * commit
   * push
   * pull/fetch

4. Add a **publish module**

   * detect publishing mode
   * invoke existing build/deploy scripts if present and appropriate
   * otherwise perform a straightforward static publish
   * surface logs and publish URL

5. Add minimal **UI flows**

   * “New Site from Template”
   * “Fork Site”
   * “Open Existing Local Workspace”
   * “Commit & Push”
   * “Publish Site”

6. Add **error handling and recovery**

   * fork still provisioning
   * repo exists already
   * no publish source configured
   * auth failure
   * push rejected
   * build failure

## Design constraints

* Keep provider-neutral core types.
* Do not make any logic GitHub-Pages-specific unless isolated behind the publish provider.
* Do not assume Cloudflare.
* Do not assume a shared server other than GitHub for this milestone.
* Do not redesign the whole editor. Focus on repo/workspace/publish plumbing.

## Suggested technical choices

* For Git operations, use either:

  * a mature Rust git library, or
  * shelling out to `git` if that is simpler and more reliable in the near term
* For GitHub API calls, use REST endpoints for:

  * create from template
  * create fork
  * repo metadata
* For credentials on macOS, prefer Keychain integration if feasible
* For site build/publish, reuse existing repo scripts when sensible rather than inventing a second build system

## Deliverables

1. Working implementation in `wiki3-app`
2. README updates documenting:

   * auth setup
   * creating from template
   * forking
   * commit/push
   * publish/update
3. A short architecture note explaining:

   * provider abstraction
   * how to add Cloudflare/R2 later
4. Basic tests for:

   * workspace metadata
   * GitHub repo creation/fork request handling
   * publish-mode detection
   * local git status/commit/push flow where testable

## Acceptance criteria

A user can:

* authenticate with GitHub
* create a new repo from `wiki3-ai-template`
* clone it locally into the app workspace
* edit files locally
* commit and push changes
* publish/update the resulting static site
* fork an existing repo and do the same from the fork

The codebase should make it obvious how to later replace:

* GitHub repo creation/fork with Cloudflare Artifacts or raw Git
* GitHub Pages publish with R2/static HTTP publish

When done, include:

* files changed
* architecture summary
* any open questions or shortcuts taken
* exact manual test steps

---

[1]: https://github.com/wiki3-ai/wiki3-app "GitHub - wiki3-ai/wiki3-app: Desktop (Mobile planned) App for running Wiki3.ai sites · GitHub"
[2]: https://github.com/wiki3-ai/wiki3-ai-site "GitHub - wiki3-ai/wiki3-ai-site: Source for Wiki3.ai site"
[3]: https://github.com/wiki3-ai/wiki3-ai-template "GitHub - wiki3-ai/wiki3-ai-template: Wiki3 AI Site Template · GitHub"
[4]: https://docs.github.com/en/repositories/creating-and-managing-repositories/creating-a-repository-from-a-template?utm_source=chatgpt.com "Creating a repository from a template"
[5]: https://docs.github.com/en/rest/repos/forks?utm_source=chatgpt.com "REST API endpoints for forks"
[6]: https://docs.github.com/en/authentication/keeping-your-account-and-data-secure/managing-your-personal-access-tokens?utm_source=chatgpt.com "Managing your personal access tokens"
[7]: https://docs.github.com/en/pages/getting-started-with-github-pages/creating-a-github-pages-site?utm_source=chatgpt.com "Creating a GitHub Pages site"
