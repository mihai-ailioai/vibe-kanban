import path from 'path';
import { fileURLToPath } from 'url';
import { defineConfig } from 'vitest/config';

const packageRoot = fileURLToPath(new URL('.', import.meta.url));

export default defineConfig({
  resolve: {
    alias: [
      {
        find: /^@\//,
        replacement: `${path.resolve(packageRoot, 'src')}/`,
      },
      {
        find: 'shared',
        replacement: path.resolve(packageRoot, '../../shared'),
      },
    ],
  },
  test: {
    environment: 'node',
    include: ['src/**/*.test.ts'],
  },
});
