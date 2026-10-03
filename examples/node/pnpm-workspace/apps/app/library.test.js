import { expect, test } from 'vitest';
import { answer } from '@fixture/library';

test('workspace source and external dependencies resolve together', () => {
  expect(answer()).toBe(42);
});
