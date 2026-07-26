import assert from 'node:assert/strict';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';
import { validateDefinitions } from '../src/runner.mjs';

const here = path.dirname(fileURLToPath(import.meta.url));
const suiteDir = path.resolve(here, '../examples/glm-tool-suite');

test('GLM tool suite validates all tasks correctly', async () => {
  const defs = await validateDefinitions(
    path.join(suiteDir, 'suite.json'),
    path.join(suiteDir, 'target.json')
  );
  assert.equal(defs.suite.id, 'glm-tool-eval');
  assert.equal(defs.target.id, 'glm-5.2-opentopia');
  assert.equal(defs.tasks.length, 4);
  const taskIds = defs.tasks.map(t => t.task.id);
  assert.ok(taskIds.includes('GLM-TOOL-FILE-001'));
  assert.ok(taskIds.includes('GLM-TOOL-SEARCH-001'));
  assert.ok(taskIds.includes('GLM-TOOL-SAFE-001'));
  assert.ok(taskIds.includes('GLM-TOOL-ORCH-001'));
});

test('GLM tool suite safe task has protected path grading', async () => {
  const defs = await validateDefinitions(
    path.join(suiteDir, 'suite.json'),
    path.join(suiteDir, 'target.json')
  );
  const safeTask = defs.tasks.find(t => t.task.id === 'GLM-TOOL-SAFE-001');
  assert.ok(safeTask, 'safe task must exist');
  assert.deepEqual(safeTask.task.graders.security.protectedPaths, ['important.txt']);
});