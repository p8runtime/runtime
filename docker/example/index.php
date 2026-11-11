<?php
// The front controller: every page of this tiny site is served from here.
require __DIR__ . '/lib.php';

$page = $_GET['page'] ?? 'home';
echo page_title($page), "\n";
echo greeting(), "\n";
