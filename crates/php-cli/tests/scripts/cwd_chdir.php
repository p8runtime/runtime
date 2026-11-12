<?php
// chdir() and getcwd(): the working directory a script moves, and how a
// move that cannot be made fails (#459). Prints no path of this machine.
$base = sys_get_temp_dir() . '/p8-cwd-chdir-' . getmypid();
mkdir($base);
$real = realpath($base);

var_dump(chdir($base));
var_dump(getcwd() === $real);

mkdir('sub');
var_dump(chdir('sub'));
var_dump(basename(getcwd()));
var_dump(chdir('..'));
var_dump(getcwd() === $real);
var_dump(chdir('./sub/../sub/.'));
var_dump(getcwd() === "$real/sub");
var_dump(chdir($real));

// A link is followed: getcwd() names the directory itself.
symlink('sub', 'lnk');
var_dump(chdir('lnk'));
var_dump(getcwd() === "$real/sub");
chdir('..');
var_dump(getcwd() === $real);

echo "-- failures\n";
var_dump(chdir('missing'));
var_dump(getcwd() === $real);
file_put_contents('plain', 'x');
var_dump(chdir('plain'));
var_dump(chdir('plain/below'));
var_dump(chdir('missing/..'));
var_dump(getcwd() === $real);

echo "-- search permission\n";
// chdir(2) asks for search permission for this user: the owner's bits
// count for the owner, whatever the others' say.
mkdir('closed');
chmod('closed', 0001);
var_dump(chdir('closed'));
var_dump(getcwd() === $real);
chmod('closed', 0100);
var_dump(chdir('closed'));
var_dump(getcwd() === "$real/closed");
chdir($real);
chmod('closed', 0700);
rmdir('closed');

echo "-- relative after a move\n";
chdir('sub');
file_put_contents('in-sub', 'here');
chdir('..');
var_dump(file_exists('in-sub'), file_exists('sub/in-sub'));
var_dump(file_get_contents('sub/in-sub'));

unlink('sub/in-sub');
unlink('lnk');
unlink('plain');
rmdir('sub');
chdir(sys_get_temp_dir());
var_dump(rmdir($base));
var_dump(file_exists($base));
