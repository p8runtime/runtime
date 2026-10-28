<?php
// Install WordPress as wp-admin/install.php's final step does.
$_SERVER['HTTP_HOST'] = 'localhost';
$_SERVER['REQUEST_URI'] = '/wp-admin/install.php?step=2';
$_SERVER['SERVER_NAME'] = 'localhost';
$_SERVER['SERVER_PORT'] = '80';
$_SERVER['REQUEST_METHOD'] = 'POST';
$_SERVER['SCRIPT_NAME'] = '/wp-admin/install.php';
$_SERVER['PHP_SELF'] = '/wp-admin/install.php';
define('WP_INSTALLING', true);
chdir(__DIR__ . '/wordpress');
require __DIR__ . '/wordpress/wp-load.php';
require ABSPATH . 'wp-admin/includes/upgrade.php';
$result = wp_install('p8 blog', 'admin', 'admin@example.com', true, '', 'password');
// The salts WordPress makes on first use, made now (the gate gives both
// installs the same ones).
foreach (['auth', 'secure_auth', 'logged_in', 'nonce'] as $scheme) {
    wp_salt($scheme);
}
echo 'user_id=', $result['user_id'], ' password=', $result['password'], "\n";
global $wpdb;
echo 'tables=', count($wpdb->get_col('SHOW TABLES')), ' options=', $wpdb->get_var("SELECT COUNT(*) FROM {$wpdb->options}"), ' posts=', $wpdb->get_var("SELECT COUNT(*) FROM {$wpdb->posts}"), "\n";
echo 'blogname=', get_option('blogname'), ' siteurl=', get_option('siteurl'), "\n";
