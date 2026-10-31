<?php
// A page given the request itself, as under php -S.
header('X-Greeting: hello');
echo htmlspecialchars($_SERVER['REQUEST_METHOD'] . ' ' . $_SERVER['REQUEST_URI'] . ' ' . ($_GET['name'] ?? ''));
