const js = require('@eslint/js');
const tsParser = require('@typescript-eslint/parser');
const tsPlugin = require('@typescript-eslint/eslint-plugin');
const tsdoc = require('eslint-plugin-tsdoc');
const globals = require('globals');

const tsRules = {
  ...js.configs.recommended.rules,
  ...tsPlugin.configs.recommended.rules,
  'no-undef': 'off',
  'no-unused-vars': 'off',
  '@typescript-eslint/no-explicit-any': 'error',
  '@typescript-eslint/no-unused-vars': [
    'error',
    {
      argsIgnorePattern: '^_',
      varsIgnorePattern: '^_',
      caughtErrorsIgnorePattern: '^_'
    }
  ]
};

module.exports = [
  {
    ignores: [
      'dist/**',
      'node_modules/**',
      'data/**',
      'docs/feedback-from-outside-llms/**',
      'markdown-rules/**',
      '.vscode/**',
      'docs/api/**'
    ]
  },
  {
    files: ['**/*.{js,cjs,mjs}'],
    languageOptions: {
      ecmaVersion: 'latest',
      sourceType: 'module',
      globals: {
        ...globals.node
      }
    },
    rules: {
      ...js.configs.recommended.rules
    }
  },
  {
    files: ['src/**/*.{ts,tsx}'],
    languageOptions: {
      parser: tsParser,
      ecmaVersion: 'latest',
      sourceType: 'module',
      globals: {
        ...globals.browser
      }
    },
    plugins: {
      '@typescript-eslint': tsPlugin,
      tsdoc
    },
    rules: {
      ...tsRules,
      'tsdoc/syntax': 'error'
    }
  },
  {
    files: ['server/**/*.ts', 'scripts/**/*.ts'],
    languageOptions: {
      parser: tsParser,
      ecmaVersion: 'latest',
      sourceType: 'module',
      globals: {
        ...globals.node
      }
    },
    plugins: {
      '@typescript-eslint': tsPlugin,
      tsdoc
    },
    rules: {
      ...tsRules,
      'tsdoc/syntax': 'error'
    }
  }
];
