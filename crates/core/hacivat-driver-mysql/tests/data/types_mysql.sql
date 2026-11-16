-- M3b Task 7 (R-M3-67): the MySQL/MariaDB type matrix golden, read by
-- tests/docker_types_m3b.rs on every MySQL-family line (mysql80, mysql84, mariadb106, mariadb1011,
-- mariadb114 on `full`).
--
-- Lines ending with `;` are setup statements, run in order. Case lines start with `=`:
-- `= <column> | <header label> | <kind> | <raw text> [| <server expression>]`; when the server
-- expression is given (`BIN()`, `HEX()`), the server's own text of it must equal the raw text
-- too (fix round 1, review m-6). The test selects the column of row 1 in
-- both protocols (binary `COM_STMT_EXECUTE`, text `COM_QUERY`) and checks the label
-- (`valfmt::type_label` of the native type), the kind (bit, geometry, or `-` for any other)
-- and the raw text (`valfmt::raw_text`: MySQL floats in the shortest round-trip form, ledger
-- 177). Text columns use `utf8mb4_bin` (collation 46 on MySQL and MariaDB) so their character
-- lengths are known on every line.
CREATE TEMPORARY TABLE hv_t07_types (id INT PRIMARY KEY, b4 BIT(4), b64 BIT(64), b1 BIT(1), g GEOMETRY, gs GEOMETRY, f FLOAT, fn FLOAT, fe FLOAT, db DOUBLE, dbig DOUBLE, dhuge DOUBLE, dsmall DOUBLE, dmax DOUBLE, d14 DECIMAL(14,2), du DECIMAL(10,2) UNSIGNED, bu BIGINT UNSIGNED, ti1 TINYINT(1), mi MEDIUMINT, iu INT UNSIGNED, y YEAR, dt DATE, dt6 DATETIME(6), ts3 TIMESTAMP(3) NULL, tm2 TIME(2), vc VARCHAR(40) COLLATE utf8mb4_bin, c5 CHAR(5) COLLATE utf8mb4_bin, tx TEXT COLLATE utf8mb4_bin, bn BINARY(3), vb VARBINARY(8), bl BLOB, en ENUM('small', 'medium') COLLATE utf8mb4_bin, st SET('a', 'b') COLLATE utf8mb4_bin) DEFAULT CHARSET = utf8mb4;
INSERT INTO hv_t07_types VALUES (1, b'0101', b'1000000000000000000000000000000000000000000000000000000000000001', b'1', ST_GeomFromText('POINT(1 1)'), ST_GeomFromText('POINT(1 1)', 4326), 0.1, -1.5, 1e-7, 0.1, 1e15, 1e21, 2.5e-10, 1.7976931348623157e308, 12345678.90, 0.50, 18446744073709551615, 1, -8388608, 4294967295, 2024, '2024-02-29', '2024-02-29 13:14:15.123456', '2024-02-29 13:14:15.123', '13:14:15.25', 'İstanbul ğüşıöç', 'ab', 'metin', X'00FF10', X'DEAD', X'BEEF', 'medium', 'a,b');
= b4 | bit(4) | bit | 0101 | LPAD(BIN(b4), 4, '0')
= b64 | bit(64) | bit | 1000000000000000000000000000000000000000000000000000000000000001 | LPAD(BIN(b64), 64, '0')
= b1 | bit(1) | bit | 1 | BIN(b1)
= g | geometry | geometry | 000000000101000000000000000000F03F000000000000F03F | HEX(g)
= gs | geometry | geometry | E61000000101000000000000000000F03F000000000000F03F | HEX(gs)
= f | float | - | 0.1
= fn | float | - | -1.5
= fe | float | - | 1e-7
= db | double | - | 0.1
= dbig | double | - | 1000000000000000
= dhuge | double | - | 1e+21
= dsmall | double | - | 2.5e-10
= dmax | double | - | 1.7976931348623157e+308
= d14 | decimal(14,2) | - | 12345678.90
= du | decimal(10,2) unsigned | - | 0.50
= bu | bigint unsigned | - | 18446744073709551615
= ti1 | tinyint(1) | - | 1
= mi | mediumint | - | -8388608
= iu | int unsigned | - | 4294967295
= y | year | - | 2024
= dt | date | - | 2024-02-29
= dt6 | datetime(6) | - | 2024-02-29 13:14:15.123456
= tm2 | time(2) | - | 13:14:15.25
= vc | varchar(40) | - | İstanbul ğüşıöç
= c5 | char(5) | - | ab
= tx | text | - | metin
= bn | binary(3) | - | \x00ff10
= vb | varbinary(8) | - | \xdead
= bl | blob | - | \xbeef
= en | enum | - | medium
= st | set | - | a,b
