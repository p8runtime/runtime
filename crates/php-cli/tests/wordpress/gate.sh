#!/bin/bash
# WordPress gate: install WordPress on MariaDB under PHP 8.4 (docker,
# with mysqli) and under p8, then make the same requests of each:
# - the public pages (home, a post, the feed, the REST API root, a search,
#   the login form) and, signed in as the admin, wp-admin's (dashboard,
#   posts, new post, settings, users, plugins);
# - three writes: a visitor's comment (which mails the moderator), the
#   admin publishing a post from the edit form, and a post sent over
#   XML-RPC with the admin's password;
# - the pages that show what they wrote;
# - each bundled block theme switched to, with the bundled plugins
#   activated, then a classic theme with popular plugins from
#   wordpress.org, and the pages they change.
# Every response and mail must be byte-identical, and so must the rows the
# writes leave, apart from the clock readings. Exit 0 when they all are.
#
# Each check is also written to apps.jsonl in the work directory (the
# p8-test-suite "Real apps" data): the apps it shows working, what it did,
# and whether it matched.
#
# Usage: crates/php-cli/tests/wordpress/gate.sh [path to php2]
# Needs docker. Works in .tmp/wordpress-gate; MariaDB listens on
# 127.0.0.1:3307 (container p8-mariadb, network p8net).
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$here/../../../.." && pwd)
php2=${1:-$repo/target/release/php2}
work=$repo/.tmp/wordpress-gate
version=7.1.4
mkdir -p "$work"
cd "$work"

installed=$(sed -n "s/^\$wp_version = '\(.*\)';/\1/p" wordpress/wp-includes/version.php 2>/dev/null || true)
if [ "$installed" != "$version" ]; then
    [ -d wordpress ] && find wordpress -delete
    curl -sSfL -o wp.tar.gz "https://wordpress.org/wordpress-$version.tar.gz"
    curl -sSfL -o wp.tar.gz.sha1 "https://wordpress.org/wordpress-$version.tar.gz.sha1"
    [ "$(shasum -a 1 wp.tar.gz | cut -d' ' -f1)" = "$(cat wp.tar.gz.sha1)" ] || { echo "wordpress-$version.tar.gz: sha1 mismatch"; exit 1; }
    tar xzf wp.tar.gz
    sed -e "s/'database_name_here'/getenv('WPDBNAME')/" -e "s/'username_here'/'wp'/" -e "s/'password_here'/'wp'/" \
        -e "s/'localhost'/getenv('WPDBHOST')/" wordpress/wp-config-sample.php > wordpress/wp-config.php
fi
# Plugins and a classic theme from wordpress.org, pinned: "path sha256".
mkdir -p dl
for pin in \
    plugin/contact-form-7.6.2.3.zip:4766cbc9b433128e43a8af9af8d97b3a7106273e743a19cd645bff5ed2141261 \
    plugin/classic-editor.1.7.0.zip:7cc7799b0b7d820fbb7528dbff8027c736e52513992416cb37279f44d3d9dd77 \
    plugin/wordpress-seo.28.8.zip:27cc01cb149db7186ef750552a5cc2ae5028933473886e419918af273b075105 \
    plugin/woocommerce.11.2.1.zip:5317434cb381b5fdc7c1dd6ee258324f08c390976aaf8d1199f2a0e9a3ab8586 \
    theme/twentytwentyone.2.9.zip:51d4ac1dcaef18b0e35a1a313bce8115d3aa4ff315bb1143f95c223c38817d8b; do
    path=${pin%%:*} sum=${pin#*:} zip=dl/$(basename "${pin%%:*}")
    [ -f "$zip" ] || curl -sSfL -o "$zip" "https://downloads.wordpress.org/$path"
    [ "$(shasum -a 256 "$zip" | cut -d' ' -f1)" = "$sum" ] || { echo "$zip: sha256 mismatch"; exit 1; }
    dir=wordpress/wp-content/${path%%/*}s
    slug=$(basename "$path" .zip)
    [ -d "$dir/${slug%%.[0-9]*}" ] || unzip -qo "$zip" -d "$dir"
done
cp "$here/page.php" "$here/install.php" "$here/setup.php" .
# A sendmail that keeps what it is given (in $MAILBOX).
printf '#!/bin/sh\ncat >> "$MAILBOX"\n' > sendmail.sh
chmod +x sendmail.sh

docker network inspect p8net >/dev/null 2>&1 || docker network create p8net >/dev/null
if ! docker ps --format '{{.Names}}' | grep -qx p8-mariadb; then
    docker rm -f p8-mariadb >/dev/null 2>&1 || true
    docker run -d --name p8-mariadb --network p8net -p 127.0.0.1:3307:3306 \
        -e MARIADB_ROOT_PASSWORD=p8root -e MARIADB_USER=wp -e MARIADB_PASSWORD=wp mariadb:11.4 >/dev/null
    until docker exec p8-mariadb mariadb -uroot -pp8root -e 'SELECT 1' >/dev/null 2>&1; do sleep 1; done
fi
image=p8-php-gate:$(shasum -a 256 "$here/php.Dockerfile" | cut -c1-12)
docker image inspect "$image" >/dev/null 2>&1 || docker build -q -t "$image" -f "$here/php.Dockerfile" "$here" >/dev/null

sql() { docker exec p8-mariadb mariadb -uroot -pp8root "$@"; }
for db in wp_php wp_p8; do
    sql -e "DROP DATABASE IF EXISTS $db; CREATE DATABASE $db; GRANT ALL ON $db.* TO 'wp'@'%';"
done
# PHP runs at the same path as p8 and as the same user: WordPress prints
# its paths, and picks how it would write files by who owns them. It also
# lists directories in the order the system gives (a theme's parts, say):
# a tmpfs lists in reverse order of creation, so the files go in reversed
# from this system's order. A request's variables (page.php reads them)
# are passed on when set; the mail sent goes to stderr.
top=/$(printf '%s' "$work" | cut -d/ -f2)
php_ref() {
    local e=() v
    for v in WPURL WPUSER WPPOST WPRAW WPNONCE WPIP WPTHEME WPPLUGINS; do
        [ -n "${!v+x}" ] && e+=(-e "$v=${!v}")
    done
    { printf '%s\n' "$1" sendmail.sh; find wordpress | tail -r; } | COPYFILE_DISABLE=1 tar -cf - --no-recursion -T - 2>/dev/null |
        docker run --rm -i --network p8net --user "$(id -u):$(id -g)" --tmpfs "$top:uid=$(id -u),gid=$(id -g),exec" \
            ${e[@]+"${e[@]}"} -e WPDBHOST=p8-mariadb -e WPDBNAME=wp_php -e MAILBOX="$work/mailbox" "$image" \
            bash -c "mkdir -p '$work' && cd '$work' && tar xf - 2>/dev/null && php -d sendmail_path='$work/sendmail.sh' $1; cat mailbox >&2 2>/dev/null || true"
}
p8() {
    local status=0
    rm -f mailbox
    MAILBOX="$work/mailbox" WPDBHOST=127.0.0.1:3307 WPDBNAME=wp_p8 "$php2" -d sendmail_path="$work/sendmail.sh" "$1" || status=$?
    cat mailbox >&2 2>/dev/null || true
    rm -f mailbox
    return $status
}
# The time of the request, which wp-admin prints: as timestamps (its user
# settings, the editor's post lock, the customizer's clock), as the clock, and as the dates of the
# posts and comments made today (the test content is dated 2026-01-01, so
# today's date is only ever the clock; the classic editor's time fields and
# post lock, and the settings' time format examples, are the clock too). A mail's Date and Message-ID, an embed's
# secret, a UUID (the customizer's changeset), the post-by-email
# suggestions and WooCommerce's share key are random, made by random_int();
# WooCommerce's install timestamp is the clock (as in its URL-encoded settings).
today=$(date -u +%Y-%m-%d)
clock() { sed -E -e 's/"time":"[0-9]+"/"time":"T"/g' -e 's/"initialServerTimestamp":[0-9]+/"initialServerTimestamp":T/g' -e 's/"activePostLock":"[0-9]+:/"activePostLock":"T:/g' -e 's#(Universal|Local) time is <code>[^<]*</code>#\1 time is <code>T</code>#g' \
    -e "s/${today}([T ])[0-9]{2}:[0-9]{2}:[0-9]{2}/${today}\\1T/g" -e 's/(_edit_lock[[:space:]])[0-9]+:/\1T:/' \
    -e 's/^Date: .*/Date: T/' -e 's/^Message-ID: <[^@]*@/Message-ID: <M@/' -e 's/(data-secret=\\?"|secret=)[A-Za-z0-9]{10}/\1S/g' \
    -e 's/[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}/UUID/g' \
    -e 's/(name="(hidden_|cur_)?(hh|mn|ss)" value=")[0-9]{2}"/\1T"/g' -e 's/(id="active_post_lock" value=")[0-9]+:/\1T:/' \
    -e 's#(random strings you could use: )<kbd>[^<]*</kbd>, <kbd>[^<]*</kbd>, <kbd>[^<]*</kbd>#\1R#' \
    -e 's#(<span class="date-time-text format-i18n">)[^<]*:[^<]*(</span>)#\1T\2#g' \
    -e 's/(install_timestamp%22%3A%22)[0-9]+/\1T/g' -e "s/${today}T[0-9]{2}%3A[0-9]{2}%3A[0-9]{2}/${today}TT/g" \
    -e 's/(woocommerce_share_key%22%3A%22)[A-Za-z0-9]+/\1S/g' "$1"; }

php_ref install.php > install.php.out 2>/dev/null
p8 install.php > install.p8.out 2>/dev/null
diff install.p8.out install.php.out || { echo "FAIL: wp_install() differs"; exit 1; }
cat install.p8.out

# The installs differ only in their dates and their generated salts (which
# nonces are made from): make those the same so the pages compare.
for db in wp_php wp_p8; do
    sql "$db" -e "UPDATE wp_posts SET post_date='2026-01-01 12:00:00' + INTERVAL ID SECOND, post_date_gmt='2026-01-01 12:00:00' + INTERVAL ID SECOND, post_modified='2026-01-01 12:00:00' + INTERVAL ID SECOND, post_modified_gmt='2026-01-01 12:00:00' + INTERVAL ID SECOND; UPDATE wp_comments SET comment_date='2026-01-01 12:00:00' + INTERVAL comment_ID SECOND, comment_date_gmt='2026-01-01 12:00:00' + INTERVAL comment_ID SECOND;"
done
sql -e "UPDATE wp_p8.wp_options o JOIN wp_php.wp_options r USING (option_name) SET o.option_value = r.option_value WHERE option_name LIKE '%\\_key' OR option_name LIKE '%\\_salt';"

failed=0
# apps.jsonl: the run, each app (name, version from its own header, kind,
# where it comes from), then a line per check naming the apps in $APPS.
json() { printf '%s' "$1" | sed -e 's/\\/\\\\/g' -e 's/"/\\"/g'; }
version_of() { tr -d '\r' < "$1" | sed -n 's/^[ \t*]*Version:[ \t]*\([^ \t]*\).*/\1/p' | head -1; }
apps_out=$work/apps.jsonl
printf '{"type":"run","p8":"%s","php":"8.4.27","wordpress":"%s"}\n' "$(git -C "$repo" rev-parse HEAD)" "$version" > "$apps_out"
# app NAME VERSION KIND SOURCE ABOUT (what it is, in a line, for someone who has not met it)
app() {
    printf '{"type":"app","name":"%s","version":"%s","kind":"%s","about":"%s","source":"%s"}\n' "$(json "$1")" "$(json "$2")" "$3" "$(json "$5")" "$(json "$4")" >> "$apps_out"
}
record() {
    local names="" n first=1 IFS=,
    for n in $APPS; do
        [ $first = 1 ] || names="$names,"
        names="$names\"$(json "$n")\""
        first=0
    done
    printf '{"type":"check","apps":[%s],"check":"%s","what":"%s","as":"%s","status":"%s","bytes":%s}\n' \
        "$names" "$(json "$1")" "$2" "$3" "$4" "$5" >> "$apps_out"
}
themes=wordpress/wp-content/themes plugins=wordpress/wp-content/plugins
app WordPress "$version" application "https://wordpress.org/wordpress-$version.tar.gz" "The website and blog platform"
app "Twenty Twenty-Five" "$(version_of $themes/twentytwentyfive/style.css)" theme https://wordpress.org/themes/twentytwentyfive/ "WordPress's default theme since 2025, built from blocks"
app "Twenty Twenty-Four" "$(version_of $themes/twentytwentyfour/style.css)" theme https://wordpress.org/themes/twentytwentyfour/ "WordPress's 2024 default theme, built from blocks"
app "Twenty Twenty-Three" "$(version_of $themes/twentytwentythree/style.css)" theme https://wordpress.org/themes/twentytwentythree/ "WordPress's 2023 default theme, built from blocks"
app "Twenty Twenty-One" "$(version_of $themes/twentytwentyone/style.css)" theme https://wordpress.org/themes/twentytwentyone/ "WordPress's 2021 default theme, a classic PHP template theme"
app Akismet "$(version_of $plugins/akismet/akismet.php)" plugin https://wordpress.org/plugins/akismet/ "Filters spam out of comments"
app "Hello Dolly" "$(version_of $plugins/hello.php)" plugin https://wordpress.org/plugins/hello-dolly/ "The sample plugin that ships with WordPress"
app "Classic Editor" "$(version_of $plugins/classic-editor/classic-editor.php)" plugin https://wordpress.org/plugins/classic-editor/ "Brings back the post editor from before blocks"
app "Contact Form 7" "$(version_of $plugins/contact-form-7/wp-contact-form-7.php)" plugin https://wordpress.org/plugins/contact-form-7/ "Contact forms that send mail"
app "Yoast SEO" "$(version_of $plugins/wordpress-seo/wp-seo.php)" plugin https://wordpress.org/plugins/wordpress-seo/ "Search-engine optimisation: titles, meta tags, sitemaps, structured data"
app WooCommerce "$(version_of $plugins/woocommerce/woocommerce.php)" plugin https://wordpress.org/plugins/woocommerce/ "An online shop: products, cart, checkout and orders"
APPS="WordPress"
install_status=pass
cmp -s install.p8.out install.php.out || install_status=fail
record "wp_install()" install admin $install_status "$(wc -c < install.p8.out | tr -d ' ')"
APPS="WordPress,Twenty Twenty-Five"
# One request under both, its variables given as NAME=value: the response
# and the mail it sent must match.
request() {
    local label=$1 name
    shift
    name=$(printf '%s' "$label" | tr -c 'a-zA-Z0-9' '_')
    (export "$@"; php_ref page.php || true) 2> "mail$name.php.txt" | clock /dev/stdin > "page$name.php.html"
    (export "$@"; p8 page.php || true) 2> "mail$name.p8.txt" | clock /dev/stdin > "page$name.p8.html"
    clock "mail$name.php.txt" > "mail$name.php.norm"
    clock "mail$name.p8.txt" > "mail$name.p8.norm"
    local what=page as=visitor bytes status=pass mailed=pass
    case "$label" in POST*) what=write ;; esac
    case " $* " in *" WPUSER=1 "*) as=admin ;; esac
    bytes=$(wc -c < "page$name.p8.html" | tr -d ' ')
    cmp -s "page$name.p8.html" "page$name.php.html" || status=fail
    cmp -s "mail$name.p8.norm" "mail$name.php.norm" || mailed=fail
    # The check's name: the request, without the phase it is grouped under
    # (the apps say that), and the pages re-read after the writes so marked.
    local check="${label#*: }"
    case "$label" in "after: "*) check="$check after the writes" ;; esac
    record "$check" $what $as $status "$bytes"
    if [ -s "mail$name.php.txt" ] || [ -s "mail$name.p8.txt" ]; then
        record "mail sent by $check" mail $as $mailed "$(wc -c < "mail$name.p8.txt" | tr -d ' ')"
    fi
    if ! cmp -s "page$name.p8.html" "page$name.php.html"; then
        echo "FAIL $label (p8 $(wc -c < "page$name.p8.html" | tr -d ' ') bytes, PHP $(wc -c < "page$name.php.html" | tr -d ' '))"
        diff <(sed 's/></>\n</g' "page$name.p8.html") <(sed 's/></>\n</g' "page$name.php.html") | head -20 || true
        failed=1
    elif ! cmp -s "mail$name.p8.norm" "mail$name.php.norm"; then
        echo "FAIL $label: the mail differs"
        diff "mail$name.p8.norm" "mail$name.php.norm" | head -20 || true
        failed=1
    else
        local mail=""
        [ -s "mail$name.p8.txt" ] && mail=", mail $(wc -c < "mail$name.p8.txt" | tr -d ' ') bytes"
        echo "PASS $label ($(wc -c < "page$name.p8.html" | tr -d ' ') bytes, sha256 $(shasum -a 256 "page$name.p8.html" | cut -c1-16)$mail)"
    fi
}

# Pages as "user:url": user 0 is signed out, 1 is the admin. They run in
# this order under both, so rows they add (wp-admin's auto-drafts) match.
for req in 0:/ 0:/?p=1 0:/?feed=rss2 0:/?rest_route=/ 0:/?s=hello 0:/wp-login.php \
    1:/wp-admin/ 1:/wp-admin/edit.php 1:/wp-admin/post-new.php 1:/wp-admin/options-general.php 1:/wp-admin/users.php 1:/wp-admin/plugins.php; do
    user=${req%%:*} url=${req#*:}
    [ "$user" = 0 ] && user=
    request "$url" WPUSER="$user" WPURL="$url"
done

# The writes. The admin publishes the draft post-new.php made above.
APPS="WordPress"
draft=$(sql -N -e "SELECT MAX(ID) FROM wp_php.wp_posts WHERE post_status='auto-draft'")
request "POST /wp-comments-post.php" WPURL=/wp-comments-post.php WPIP=192.0.2.10 \
    WPPOST='comment_post_ID=1&author=Ada&email=ada%40example.com&url=&comment=Hello+from+a+visitor.&submit=Post+Comment&comment_parent=0'
request "POST /wp-admin/post.php" WPURL=/wp-admin/post.php WPUSER=1 WPNONCE="update-post_$draft" \
    WPPOST="action=editpost&post_ID=$draft&post_type=post&post_author=1&original_post_status=auto-draft&post_title=Hello+from+p8&content=%3C%21--+wp%3Aparagraph+--%3E%3Cp%3EWritten+under+%3Cstrong%3Ep8%3C%2Fstrong%3E.%3C%2Fp%3E%3C%21--+%2Fwp%3Aparagraph+--%3E&excerpt=&post_category%5B%5D=1&tax_input%5Bpost_tag%5D=rust%2C+php&comment_status=open&ping_status=open&visibility=public&publish=Publish&_wp_http_referer=%2Fwp-admin%2Fpost-new.php"
request "POST /xmlrpc.php wp.newPost" WPURL=/xmlrpc.php \
    WPRAW='<?xml version="1.0"?><methodCall><methodName>wp.newPost</methodName><params><param><value><int>1</int></value></param><param><value><string>admin</string></value></param><param><value><string>password</string></value></param><param><value><struct><member><name>post_title</name><value><string>Posted over XML-RPC</string></value></member><member><name>post_content</name><value><string>Sent &lt;em&gt;remotely&lt;/em&gt;.</string></value></member><member><name>post_status</name><value><string>publish</string></value></member><member><name>terms_names</name><value><struct><member><name>post_tag</name><value><array><data><value><string>xml-rpc</string></value></data></array></value></member></struct></value></member></struct></value></param></params></methodCall>'

# The rows the writes left.
rows() {
    sql -N "$1" -e "SELECT * FROM wp_posts ORDER BY ID; SELECT * FROM wp_postmeta ORDER BY meta_id; SELECT * FROM wp_comments ORDER BY comment_ID;
        SELECT * FROM wp_commentmeta ORDER BY meta_id; SELECT * FROM wp_terms ORDER BY term_id; SELECT * FROM wp_term_taxonomy ORDER BY term_taxonomy_id;
        SELECT * FROM wp_term_relationships ORDER BY object_id, term_taxonomy_id;" | clock /dev/stdin
}
rows wp_php > rows.php.txt
rows wp_p8 > rows.p8.txt
record "rows the writes left (posts, meta, comments, terms)" database admin "$(cmp -s rows.p8.txt rows.php.txt && echo pass || echo fail)" "$(wc -c < rows.p8.txt | tr -d ' ')"
if cmp -s rows.p8.txt rows.php.txt; then
    echo "PASS the database ($(wc -l < rows.p8.txt | tr -d ' ') rows of posts, meta, comments and terms)"
else
    echo "FAIL the database differs"
    diff rows.p8.txt rows.php.txt | head -20 || true
    failed=1
fi

# The pages that show them, with the writes' times (the clock) pinned: a
# second apart, by ID, so pages listing by date never meet a tie (MariaDB
# orders tied rows as it likes, and differently from run to run).
APPS="WordPress,Twenty Twenty-Five"
for db in wp_php wp_p8; do
    sql "$db" -e "UPDATE wp_posts SET post_date='2026-01-02 12:00:00' + INTERVAL ID SECOND, post_date_gmt='2026-01-02 12:00:00' + INTERVAL ID SECOND, post_modified='2026-01-02 12:00:00' + INTERVAL ID SECOND, post_modified_gmt='2026-01-02 12:00:00' + INTERVAL ID SECOND WHERE post_modified_gmt >= '$today';
        UPDATE wp_comments SET comment_date='2026-01-02 12:00:00' + INTERVAL comment_ID SECOND, comment_date_gmt='2026-01-02 12:00:00' + INTERVAL comment_ID SECOND WHERE comment_date_gmt >= '$today';"
done
for req in 0:/ 0:/?p=1 0:/?p=$draft 0:/?feed=rss2 0:/?tag=xml-rpc 1:/wp-admin/edit-comments.php 1:/wp-admin/edit.php \
    0:/?cat=1 0:/?tag=rust 0:/?author=1 0:/?p=9999 0:/?m=2026 0:/?page_id=2 0:/?feed=comments-rss2 0:/?feed=atom \
    '0:/?s=p8&post_type=post' '0:/?p=1&replytocom=1' 0:/wp-login.php?action=lostpassword \
    0:/wp-sitemap.xml 0:/?sitemap=index '0:/?sitemap=posts&sitemap-subtype=post&paged=1' \
    0:/?rest_route=/wp/v2/posts 0:/?rest_route=/wp/v2/users 0:/?rest_route=/wp/v2/comments 0:/?rest_route=/wp/v2/categories \
    0:/?rest_route=/wp/v2/pages/2 0:/?rest_route=/wp/v2/types '0:/?rest_route=/oembed/1.0/embed&url=http%3A%2F%2Flocalhost%2F%3Fp%3D1'; do
    user=${req%%:*} url=${req#*:}
    [ "$user" = 0 ] && user=
    request "after: $url" WPUSER="$user" WPURL="$url"
done

# Themes and plugins, switched and activated by WordPress under each.
setup() {
    local label=$1
    shift
    (export "$@"; php_ref setup.php || true) > setup.php.out 2>/dev/null
    (export "$@"; p8 setup.php || true) > setup.p8.out 2>/dev/null
    record "$label" activate admin "$(cmp -s setup.p8.out setup.php.out && echo pass || echo fail)" "$(wc -c < setup.p8.out | tr -d ' ')"
    if cmp -s setup.p8.out setup.php.out; then
        echo "PASS $label: $(tr '\n' ';' < setup.p8.out)"
    else
        echo "FAIL $label"
        diff setup.p8.out setup.php.out | head -10 || true
        failed=1
    fi
}
APPS="Twenty Twenty-Four,Hello Dolly,Akismet"
setup "switch to Twenty Twenty-Four, activate Hello Dolly and Akismet" WPTHEME=twentytwentyfour WPPLUGINS=hello.php,akismet/akismet.php
for req in 0:/ 0:/?p=1 0:/?cat=1 0:/?page_id=2 0:/?p=9999 1:/wp-admin/ 1:/wp-admin/plugins.php 1:/wp-admin/themes.php \
    '1:/wp-admin/options-general.php?page=akismet-key-config' 1:/wp-admin/site-editor.php 1:/wp-admin/customize.php; do
    user=${req%%:*} url=${req#*:}
    [ "$user" = 0 ] && user=
    request "twentytwentyfour: $url" WPUSER="$user" WPURL="$url"
done
APPS="Twenty Twenty-Three"
setup "switch to Twenty Twenty-Three" WPTHEME=twentytwentythree
for req in 0:/ 0:/?p=1 0:/?page_id=2 0:/?s=hello 1:/wp-admin/site-editor.php; do
    user=${req%%:*} url=${req#*:}
    [ "$user" = 0 ] && user=
    request "twentytwentythree: $url" WPUSER="$user" WPURL="$url"
done
APPS="Twenty Twenty-One,Classic Editor,Contact Form 7"
setup "switch to Twenty Twenty-One, activate Classic Editor and Contact Form 7" WPTHEME=twentytwentyone \
    WPPLUGINS=classic-editor/classic-editor.php,contact-form-7/wp-contact-form-7.php
for req in 0:/ 0:/?p=1 0:/?page_id=2 0:/?cat=1 0:/?p=9999 1:/wp-admin/ 1:/wp-admin/post-new.php '1:/wp-admin/post.php?post=1&action=edit' \
    '1:/wp-admin/admin.php?page=wpcf7' '1:/wp-admin/admin.php?page=wpcf7-new' 1:/wp-admin/options-writing.php; do
    user=${req%%:*} url=${req#*:}
    [ "$user" = 0 ] && user=
    request "twentytwentyone: $url" WPUSER="$user" WPURL="$url"
done
APPS="Yoast SEO"
setup "activate Yoast SEO" WPPLUGINS=wordpress-seo/wp-seo.php
for req in 0:/ 0:/?p=1 0:/?cat=1 0:/?page_id=2 0:/?author=1 0:/?sitemap=index 0:/?feed=rss2 \
    1:/wp-admin/ '1:/wp-admin/admin.php?page=wpseo_dashboard' '1:/wp-admin/post.php?post=1&action=edit' 1:/wp-admin/edit.php; do
    user=${req%%:*} url=${req#*:}
    [ "$user" = 0 ] && user=
    request "yoast: $url" WPUSER="$user" WPURL="$url"
done
APPS="WooCommerce"
setup "activate WooCommerce" WPPLUGINS=woocommerce/woocommerce.php
for req in 0:/ 0:/?post_type=product 0:/?p=1 '0:/?rest_route=/wc/store/v1/products' \
    1:/wp-admin/ '1:/wp-admin/edit.php?post_type=product' '1:/wp-admin/post-new.php?post_type=product' \
    '1:/wp-admin/admin.php?page=wc-settings' '1:/wp-admin/edit.php?post_type=shop_order'; do
    user=${req%%:*} url=${req#*:}
    [ "$user" = 0 ] && user=
    request "woocommerce: $url" WPUSER="$user" WPURL="$url"
done
exit $failed
