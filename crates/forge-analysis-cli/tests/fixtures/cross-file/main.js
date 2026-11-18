import { run } from './runner.js';
import { gone } from './gone.js';
import _ from 'lodash';
import { missing } from './runner.js';
run('ls');
setTimeout(() => missing(), 100);
