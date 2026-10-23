<?php
$w = new XMLWriter();
try { $w->startElement('x'); } catch (Error $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
$w->openMemory();
try { $w->startElement('1bad'); } catch (Error $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
try { xmlwriter_start_element($w, '1bad'); } catch (Error $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
try { $w->writeElementNs('p', '1bad', 'u'); } catch (Error $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
try { xmlwriter_write_element_ns($w, 'p', '1bad', 'u'); } catch (Error $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
try { $w->openUri(''); } catch (Error $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
var_dump($w->openUri('/nonexistent/dir/x.xml'));
try { clone $w; } catch (Error $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
var_dump($w);
var_dump(xmlwriter_open_memory());
$w->openMemory();
$w->startDocument('1.0', 'ISO-8859-1');
$w->startElement('a'); $w->writeAttribute('t', "é\t\n<\"&>'"); $w->text("é € <&>\"'\r"); $w->endElement();
$w->endDocument();
echo $w->outputMemory();
$w->openMemory(); $w->startDocument(null, 'utf-8', 'yes'); $w->writeElement('é', 'ü'); $w->writeAttribute('x','y'); var_dump($w->outputMemory(false)); var_dump($w->flush());
$w=new XMLWriter;$w->openMemory();$w->startElement("a");$w->writeAttribute("t","é€😀�");$w->endElement();echo $w->outputMemory(),"
"; foreach(["utf8","UTF-8","latin1","ISO-8859-1","ascii","US-ASCII","shift_jis","bogus"] as $e){$w->openMemory(); var_dump(@$w->startDocument("1.0",$e)); echo $w->outputMemory();}

// Indentation, namespaces, a DTD, PIs and CDATA.
$w = new XMLWriter();
$w->openMemory();
$w->setIndent(true);
$w->setIndentString('  ');
$w->startDocument('1.0', 'UTF-8');
$w->startDtd('root', '-//X//DTD', 'x.dtd');
$w->writeDtdElement('root', '(item)*');
$w->writeDtdAttlist('item', 'id CDATA #IMPLIED');
$w->writeDtdEntity('ent', 'value');
$w->endDtd();
$w->startElementNs('p', 'root', 'urn:p');
$w->writeAttributeNs('q', 'attr', 'urn:q', 'v');
$w->writePi('php', 'echo 1;');
$w->startElement('item');
$w->writeCdata('<raw>');
$w->endElement();
$w->writeElement('empty');
$w->startElement('full');
$w->fullEndElement();
$w->writeComment(' c ');
$w->endDocument();
echo $w->outputMemory();
var_dump(xmlwriter_start_element($w, 'after'), xmlwriter_end_element($w), xmlwriter_output_memory($w));
