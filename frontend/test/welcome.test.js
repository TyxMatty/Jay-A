// The welcome tour: every step has its scene and its Portuguese text; navigation stays in range.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { WELCOME_STEPS, clampStep, shouldShowWelcome } from '../src/core/welcome.js';
import { WELCOME_ART } from '../src/ui/welcome-art.js';
import ptBR from '../src/locales/pt-BR.js';

test('every step has its scene and its translation', () => {
  for (const step of WELCOME_STEPS) {
    assert.match(WELCOME_ART[step.art] ?? '', /^<svg/, step.key);
    assert.ok(ptBR[step.title], `pt-BR title for ${step.key}`);
    assert.ok(ptBR[step.body], `pt-BR text for ${step.key}`);
  }
});

test('navigation stays inside the tour, and it opens once', () => {
  assert.equal(clampStep(-3), 0);
  assert.equal(clampStep(99), WELCOME_STEPS.length - 1);
  assert.equal(clampStep(Number.NaN), 0);
  assert.equal(shouldShowWelcome({}), true);
  assert.equal(shouldShowWelcome({ welcomeSeen: true }), false);
});
