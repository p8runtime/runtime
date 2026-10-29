<?php
// DOMDocument::schemaValidateSource against PHPUnit's own schemas (what PHPUnit 9 and 11 do with phpunit.xml).
$base = __DIR__ . '/dom_schema_phpunit';
$xsds = [
    '11.5' => file_get_contents("$base/11.5.xsd"),
    '9.6' => file_get_contents("$base/9.6.xsd"),
    '9.2' => file_get_contents("$base/9.2.xsd"),
];
function v($xml, $xsd) {
    libxml_use_internal_errors(true);
    libxml_clear_errors();
    $d = new DOMDocument;
    $d->loadXML($xml);
    $r = $d->schemaValidateSource($xsd);
    echo $r ? "VALID\n" : "INVALID\n";
    foreach (libxml_get_errors() as $e) {
        echo "  L{$e->level} C{$e->code} col{$e->column} line{$e->line} ", rtrim($e->message), "\n";
    }
}
$variants = [
'minimal' => '<phpunit/>',
'wp92' => <<<'XML'
<phpunit
		xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
		xsi:noNamespaceSchemaLocation="http://schema.phpunit.de/9.2/phpunit.xsd"
		bootstrap="tests/phpunit/includes/bootstrap.php"
		backupGlobals="false"
		colors="true"
		beStrictAboutTestsThatDoNotTestAnything="true"
		beStrictAboutOutputDuringTests="true"
		failOnRisky="true"
		convertErrorsToExceptions="true"
		convertWarningsToExceptions="true"
		convertNoticesToExceptions="true"
		convertDeprecationsToExceptions="true"
		>
	<testsuites>
		<!-- Default test suite to run all tests. -->
		<testsuite name="default">
			<directory suffix=".php">tests/phpunit/tests</directory>
			<exclude>tests/phpunit/tests/rest-api/rest-autosaves-controller.php</exclude>
		</testsuite>
		<!-- Sets the DOING_AUTOSAVE constant, so needs to be run last. -->
		<testsuite name="restapi-autosave">
			<file>tests/phpunit/tests/rest-api/rest-autosaves-controller.php</file>
		</testsuite>
	</testsuites>
	<groups>
		<exclude>
			<group>ajax</group>
			<group>ms-files</group>
			<group>ms-required</group>
			<group>external-http</group>
			<group>html-api-web-platform-tests</group>
		</exclude>
	</groups>
	<logging>
		<log type="junit" target="tests/phpunit/build/logs/junit.xml" />
	</logging>
	<php>
		<const name="WP_RUN_CORE_TESTS" value="1" />
	</php>
	<filter>
		<whitelist addUncoveredFilesFromWhitelist="true">
			<directory suffix=".php">src</directory>
			<exclude>
				<!-- Third party library exclusions. -->
				<directory suffix=".php">src/wp-includes/ID3</directory>
				<directory suffix=".php">src/wp-includes/IXR</directory>
				<directory suffix=".php">src/wp-includes/PHPMailer</directory>
				<directory suffix=".php">src/wp-includes/Requests</directory>
				<directory suffix=".php">src/wp-includes/php-ai-client</directory>
				<directory suffix=".php">src/wp-includes/SimplePie</directory>
				<directory suffix=".php">src/wp-includes/sodium_compat</directory>
				<directory suffix=".php">src/wp-includes/Text</directory>

				<!-- Plugins and themes. -->
				<directory suffix=".php">src/wp-content/</directory>

				<file>src/wp-admin/includes/class-ftp*</file>
				<file>src/wp-admin/includes/class-pclzip.php</file>
				<file>src/wp-admin/includes/deprecated.php</file>
				<file>src/wp-admin/includes/ms-deprecated.php</file>

				<file>src/wp-includes/atomlib.php</file>
				<file>src/wp-includes/class-IXR.php</file>
				<file>src/wp-includes/class-json.php</file>
				<file>src/wp-includes/class-phpass.php</file>
				<file>src/wp-includes/class-pop3.php</file>
				<file>src/wp-includes/class-requests.php</file>
				<file>src/wp-includes/class-simplepie.php</file>
				<file>src/wp-includes/class-snoopy.php</file>
				<file>src/wp-includes/deprecated.php</file>
				<file>src/wp-includes/ms-deprecated.php</file>
				<file>src/wp-includes/pluggable-deprecated.php</file>
				<file>src/wp-includes/rss.php</file>
			</exclude>
		</whitelist>
	</filter>
</phpunit>
XML,
'typical' => '<?xml version="1.0" encoding="UTF-8"?>
<phpunit xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
         xsi:noNamespaceSchemaLocation="https://schema.phpunit.de/11.5/phpunit.xsd"
         bootstrap="vendor/autoload.php"
         colors="true"
         cacheDirectory=".phpunit.cache"
         executionOrder="depends,defects"
         columns="max"
         failOnRisky="true">
    <testsuites>
        <testsuite name="unit">
            <directory>tests/Unit</directory>
            <file>tests/a.php</file>
            <exclude>tests/x</exclude>
        </testsuite>
    </testsuites>
    <source>
        <include>
            <directory suffix=".php">src</directory>
        </include>
        <exclude><file>src/x.php</file></exclude>
    </source>
    <coverage>
        <report><html outputDirectory="build/html"/><clover outputFile="c.xml"/></report>
    </coverage>
    <php>
        <ini name="memory_limit" value="-1"/>
        <env name="APP_ENV" value="testing"/>
        <const name="X" value="1"/>
        <var name="a" value="b"/>
    </php>
    <extensions><bootstrap class="Foo"><parameter name="a" value="b"/></bootstrap></extensions>
    <logging><junit outputFile="j.xml"/></logging>
    <groups><include><group>a</group></include></groups>
</phpunit>',
'unknown_attr' => '<phpunit foo="bar" colors="maybe" columns="abc"/>',
'unknown_child' => '<phpunit><foo/></phpunit>',
'bad_order' => '<phpunit><php><env name="a"/></php><testsuites><testsuite/></testsuites></phpunit>',
'dup_child' => '<phpunit><source/><source/></phpunit>',
'text' => '<phpunit>hello<testsuites><testsuite name="a">oops</testsuite></testsuites></phpunit>',
'testsuites_empty' => '<phpunit><testsuites/></phpunit>',
'bad_enum' => '<phpunit executionOrder="sideways" bootstrap="a b" cacheDirectory="a%zz"/>',
'wrong_root' => '<foo/>',
'groups_bad' => '<phpunit><groups/></phpunit>',
'groups_two' => '<phpunit><groups><include><group>a</group></include><exclude><group>b</group></exclude></groups></phpunit>',
'ints' => '<phpunit shortenArraysForExportThreshold="x" timeoutForSmallTests="1.5" stderr="yes" backupGlobals="0"/>',
'multiline' => "<phpunit\n  colors=\"x\"\n  foo=\"1\">\n  <testsuites>\n    <bogus/>\n  </testsuites>\n</phpunit>",

'old9' => '<phpunit backupGlobals="false" backupStaticAttributes="false" bootstrap="b.php" colors="true" convertErrorsToExceptions="true" processIsolation="false" stopOnFailure="false" syntaxCheck="false" verbose="true">
  <testsuites><testsuite name="a"><directory phpVersion="7.4" phpVersionOperator="&gt;=" suffix="Test.php">t</directory><file>x.php</file><exclude>y</exclude></testsuite></testsuites>
  <filter><whitelist processUncoveredFilesFromWhitelist="true"><directory suffix=".php">src</directory><exclude><directory>src/x</directory></exclude></whitelist></filter>
  <logging><log type="coverage-html" target="x" lowUpperBound="35"/><log type="junit" target="j.xml"/></logging>
  <listeners><listener class="X" file="y.php"><arguments><string>a</string></arguments></listener></listeners>
</phpunit>',
'coverage9' => '<phpunit><coverage processUncoveredFiles="true" includeUncoveredFiles="true" cacheDirectory="c"><include><directory suffix=".php">src</directory></include><exclude><file>a</file></exclude><report><html outputDirectory="h" lowUpperBound="50" highLowerBound="90"/><text outputFile="php://stdout" showUncoveredFiles="false"/><clover outputFile="c.xml"/></report></coverage></phpunit>',
'extensions' => '<phpunit><extensions><extension class="x"/><bootstrap class="y"/></extensions><php><includePath>a</includePath><ini name="a" value="b"/><ini name="c"/><get name="g" value="1"/><post name="p" value="1"/><server name="s" value="1"/><request name="r" value="1" force="true"/></php></phpunit>',
'default_ns' => '<phpunit xmlns="http://example.org/ns"><testsuites/></phpunit>',
'comments_pi' => '<?xml version="1.0"?><!DOCTYPE phpunit [<!ENTITY e "x">]><phpunit><!-- c --><?pi data?><testsuites><testsuite name="&e;"/></testsuites></phpunit>',
'whitespace' => "<phpunit>\n\t<testsuites>\n\t\t<testsuite name=\"a\">\n\t\t</testsuite>\n\t</testsuites>\n</phpunit>\n",
'bad_bool' => '<phpunit colors="TRUE" failOnWarning="1" stopOnFailure="0" cacheResult="  true  "/>',
'dup_testsuite_attr' => '<phpunit><testsuites><testsuite name="a" name2="b"/><testsuite/></testsuites><testsuite name="x"/></phpunit>',
'mixed_all' => '<phpunit><testsuites><testsuite name="a"/></testsuites><testsuites><testsuite name="a"/></testsuites><coverage/><source/></phpunit>',
'text_in_leaf' => '<phpunit><source><include><directory suffix=".php" prefix="p" bogus="1">src</directory><file>a</file><x/></include></source></phpunit>',
'php_children' => '<phpunit><php><env/><const value="1"/><var name="x" value="y" extra="z"/><unknown/></php></phpunit>',
'logging11' => '<phpunit><logging><junit outputFile="a"/><junit outputFile="b"/><teamcity/><testdoxHtml outputFile="x"/></logging></phpunit>',
'groups_empty_include' => '<phpunit><groups><include/></groups></phpunit>',
'groups_dup' => '<phpunit><groups><exclude><group>a</group></exclude><include><group>b</group></include></groups></phpunit>',
];
foreach ($xsds as $ver => $xsd) {
    foreach ($variants as $name => $xml) {
        echo "== $ver $name\n";
        v($xml, $xsd);
    }
}
