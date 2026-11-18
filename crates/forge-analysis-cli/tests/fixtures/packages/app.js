import React from 'react';
import ink from 'ink';
import path from 'node:path';
function view() { return React.createElement('div'); }
function state() { return React.useState(0); }
ink.render(view());
path.join('a', 'b');
