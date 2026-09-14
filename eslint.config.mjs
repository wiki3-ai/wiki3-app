// @ts-check
import js from '@eslint/js';
import globals from 'globals';
import tseslint from 'typescript-eslint';

export default tseslint.config(
  {
    // `dist` is build output and `src-tauri` is Rust. `src/public` holds the
    // container-engine bundle: a ~4 MB artifact produced by `deno task build`
    // in devcontainers-cli and copied in verbatim, so linting it would report
    // style decisions nobody here made and take a while doing it.
    ignores: ['dist/**', 'node_modules/**', 'src-tauri/**', 'src/public/**'],
  },

  js.configs.recommended,
  ...tseslint.configs.recommendedTypeChecked,

  {
    languageOptions: {
      parserOptions: {
        // The type-aware rules need a program. `projectService` picks the
        // right tsconfig per file rather than being handed a list of paths.
        // This config file is JavaScript and so belongs to no tsconfig.
        projectService: {
          allowDefaultProject: ['eslint.config.mjs'],
        },
        tsconfigRootDir: import.meta.dirname,
      },
      // The `src/` code runs in a WebView.
      globals: { ...globals.browser },
    },
    rules: {
      // A leading underscore is how this codebase marks something as
      // deliberately unused — the same convention the Rust side uses, where
      // the compiler enforces it.
      '@typescript-eslint/no-unused-vars': [
        'error',
        {
          argsIgnorePattern: '^_',
          varsIgnorePattern: '^_',
          caughtErrorsIgnorePattern: '^_',
        },
      ],
    },
  },

  {
    // Config files and the test suite run on Node, not in the WebView.
    files: ['*.config.ts', 'tests/**/*.ts'],
    languageOptions: {
      globals: { ...globals.node },
    },
  },
);
