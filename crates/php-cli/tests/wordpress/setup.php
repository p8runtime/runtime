<?php
// Change the site as wp-admin would, then report it: WPTHEME=<slug>
// switches the theme (switch_theme), WPPLUGINS=<a/a.php,b.php> activates
// those plugins (activate_plugin), as the admin.
$_SERVER['HTTP_HOST'] = 'localhost';
$_SERVER['REQUEST_URI'] = '/wp-admin/';
$_SERVER['SERVER_NAME'] = 'localhost';
$_SERVER['SERVER_PORT'] = '80';
$_SERVER['REQUEST_METHOD'] = 'GET';
define('WP_HTTP_BLOCK_EXTERNAL', true);
define('DISABLE_WP_CRON', true);
define('WP_ADMIN', true);
chdir(__DIR__ . '/wordpress/wp-admin');
require __DIR__ . '/wordpress/wp-load.php';
require_once ABSPATH . 'wp-admin/includes/admin.php';
wp_set_current_user(1);
if ($theme = getenv('WPTHEME')) {
    $t = wp_get_theme($theme);
    echo 'theme ', $theme, ': ', $t->exists() ? 'exists' : 'missing', ', errors ', var_export($t->errors() ? $t->errors()->get_error_codes() : [], true), "\n";
    switch_theme($theme);
    echo 'now ', get_stylesheet(), ' (', wp_get_theme()->get('Name'), ' ', wp_get_theme()->get('Version'), ")\n";
}
foreach (array_filter(explode(',', (string) getenv('WPPLUGINS'))) as $plugin) {
    $r = activate_plugin($plugin);
    echo 'plugin ', $plugin, ': ', is_wp_error($r) ? 'error ' . $r->get_error_message() : 'active', "\n";
}
echo 'active plugins: ', implode(', ', get_option('active_plugins')), "\n";
