<?php
$db = new PDO('sqlite::memory:');
$db->setAttribute(PDO::ATTR_ERRMODE, PDO::ERRMODE_EXCEPTION);
$db->exec('create table p (id integer primary key, price integer, qty integer, total integer generated always as (price * qty) virtual, label text as (\'#\' || id))');
$db->exec('insert into p (price, qty) values (3, 4), (5, 6)');
var_dump($db->query('select id, total, label from p order by id')->fetchAll(PDO::FETCH_ASSOC));
$db->exec('create view big as select id, total from p where total > 20');
var_dump($db->query('select * from big')->fetchAll(PDO::FETCH_ASSOC));
$db->exec('update p set qty = 10 where id = 1');
var_dump($db->query('select total from p where id = 1')->fetchColumn());
