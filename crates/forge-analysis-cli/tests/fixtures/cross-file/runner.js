import { exec } from 'node:child_process';
export function run(cmd) { exec(cmd); }
export { helper } from './helper.js';
