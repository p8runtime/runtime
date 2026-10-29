<?php
function v($xml, $xsd, $flags = 0) {
    libxml_use_internal_errors(true);
    libxml_clear_errors();
    $d = new DOMDocument; $d->loadXML($xml);
    $r = $d->schemaValidateSource($xsd, $flags);
    $out = [];
    foreach (libxml_get_errors() as $e) {
        $out[] = preg_replace("/^Element '[^']*': /", '', rtrim($e->message));
    }
    echo $r ? "ok\n" : "  " . implode(" | ", $out) . "\n";
}
function t($title, $model, $cases) {
    echo "##### $title\n";
    $xsd = '<?xml version="1.0"?><xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema"><xs:element name="r"><xs:complexType>' . $model . '</xs:complexType></xs:element></xs:schema>';
    foreach ($cases as $c) { echo "$c => "; v($c, $xsd); }
}
$E = fn($n, $extra = '') => "<xs:element name=\"$n\" $extra/>";
t('a* z', '<xs:sequence>' . $E('a', 'minOccurs="0" maxOccurs="unbounded"') . $E('z') . '</xs:sequence>', ['<r/>', '<r><a/></r>', '<r><a/><a/><c/></r>', '<r><c/></r>', '<r><z/><z/></r>', '<r><a/><z/><a/></r>']);
t('a+ z', '<xs:sequence>' . $E('a', 'maxOccurs="unbounded"') . $E('z') . '</xs:sequence>', ['<r/>', '<r><a/></r>', '<r><a/><a/><c/></r>', '<r><c/></r>', '<r><z/></r>', '<r><a/><z/><a/></r>']);
t('a+ only', '<xs:sequence>' . $E('a', 'maxOccurs="unbounded"') . '</xs:sequence>', ['<r/>', '<r><a/></r>', '<r><a/><c/></r>', '<r><c/></r>']);
t('a* only', '<xs:sequence>' . $E('a', 'minOccurs="0" maxOccurs="unbounded"') . '</xs:sequence>', ['<r/>', '<r><a/><c/></r>', '<r><c/></r>']);
t('a{1,3} z', '<xs:sequence>' . $E('a', 'maxOccurs="3"') . $E('z') . '</xs:sequence>', ['<r/>', '<r><a/></r>', '<r><a/><a/><a/><a/></r>', '<r><a/><c/></r>']);
t('a? z', '<xs:sequence>' . $E('a', 'minOccurs="0"') . $E('z') . '</xs:sequence>', ['<r/>', '<r><a/></r>', '<r><c/></r>', '<r><a/><c/></r>', '<r><a/><a/></r>']);
t('(a,b)+ z', '<xs:sequence><xs:sequence maxOccurs="unbounded">' . $E('a') . $E('b') . '</xs:sequence>' . $E('z') . '</xs:sequence>', ['<r/>', '<r><a/></r>', '<r><a/><b/></r>', '<r><a/><b/><c/></r>', '<r><a/><b/><a/></r>', '<r><a/><c/></r>']);
t('(a|b)+ z', '<xs:sequence><xs:choice maxOccurs="unbounded">' . $E('a') . $E('b') . '</xs:choice>' . $E('z') . '</xs:sequence>', ['<r/>', '<r><a/></r>', '<r><a/><b/></r>', '<r><a/><b/><c/></r>', '<r><c/></r>']);
t('choice once', '<xs:choice>' . $E('a') . $E('b') . '</xs:choice>', ['<r/>', '<r><c/></r>', '<r><a/><b/></r>', '<r><a/><a/></r>']);
t('choice opt then z', '<xs:sequence><xs:choice minOccurs="0">' . $E('a') . $E('b') . '</xs:choice>' . $E('z') . '</xs:sequence>', ['<r/>', '<r><c/></r>', '<r><a/><c/></r>', '<r><a/></r>']);
t('a b? c', '<xs:sequence>' . $E('a') . $E('b', 'minOccurs="0"') . $E('c') . '</xs:sequence>', ['<r/>', '<r><a/></r>', '<r><a/><b/></r>', '<r><a/><x/></r>', '<r><a/><b/><x/></r>', '<r><a/><b/><b/></r>']);
t('a b* c', '<xs:sequence>' . $E('a') . $E('b', 'minOccurs="0" maxOccurs="unbounded"') . $E('c') . '</xs:sequence>', ['<r><a/></r>', '<r><a/><b/></r>', '<r><a/><x/></r>', '<r><a/><b/><x/></r>', '<r><a/><b/><b/><x/></r>', '<r><a/><c/><b/></r>']);
t('a b+ c+', '<xs:sequence>' . $E('a') . $E('b', 'maxOccurs="unbounded"') . $E('c', 'maxOccurs="unbounded"') . '</xs:sequence>', ['<r><a/></r>', '<r><a/><b/></r>', '<r><a/><b/><c/><x/></r>', '<r><a/><b/><c/><b/></r>', '<r><a/><x/></r>']);
t('a{2,} ', '<xs:sequence>' . $E('a', 'minOccurs="2" maxOccurs="unbounded"') . '</xs:sequence>', ['<r/>', '<r><a/></r>', '<r><a/><a/><x/></r>']);
t('nested choice-seq', '<xs:choice><xs:sequence>' . $E('a') . $E('b') . '</xs:sequence><xs:sequence>' . $E('c') . $E('d') . '</xs:sequence></xs:choice>', ['<r/>', '<r><a/></r>', '<r><c/></r>', '<r><a/><x/></r>', '<r><a/><d/></r>', '<r><x/></r>']);
t('choice unbounded only', '<xs:choice minOccurs="0" maxOccurs="unbounded">' . $E('a') . $E('b') . '</xs:choice>', ['<r><a/><x/></r>', '<r><x/></r>', '<r><a/><b/><a/><x/></r>']);
t('choice unbounded in seq w/ opt', '<xs:sequence><xs:choice minOccurs="0" maxOccurs="unbounded">' . $E('a') . $E('b') . '</xs:choice>' . $E('z', 'minOccurs="0"') . '</xs:sequence>', ['<r><a/><x/></r>', '<r><x/></r>', '<r><z/><x/></r>', '<r><z/><a/></r>']);
