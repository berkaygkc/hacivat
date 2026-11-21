-- M3b Task 7 (R-M3-67): the PostgreSQL type matrix golden, read by tests/docker_types_m3b.rs.
--
-- One case per line: `<expression> -- <kind> => <expected raw text>`. The test runs
-- `SELECT <expression>` through the typed (binary where decoded) path on every PostgreSQL line
-- (14 to 18), checks the column's logical kind and the cell's raw text against the expected
-- text here, and checks the same raw text against the server's own text of the expression (its
-- output function, as a simple query returns it).
-- The session runs with TimeZone UTC, lc_monetary C, DateStyle ISO and IntervalStyle postgres
-- (set by the test), so the server's text is deterministic. Kinds: array, network, bit, money,
-- xml, composite, range, geometry, opaque. Lines starting with `--` are comments.
--
-- Arrays (binary): lower bounds, dimensions, NULLs, quoting, every M3a scalar.
'[0:2]={1,2,3}'::int4[] -- array => [0:2]={1,2,3}
'{{1,NULL},{3,4}}'::int4[] -- array => {{1,NULL},{3,4}}
'[1:1][-1:0]={{5,6}}'::int4[] -- array => [1:1][-1:0]={{5,6}}
'{{{1,2}},{{3,4}}}'::int4[] -- array => {{{1,2}},{{3,4}}}
'{}'::int4[] -- array => {}
ARRAY['a b', '"', '', 'NULL', 'x\y', 'şİı', 'tab	in']::text[] -- array => {"a b","\"","","NULL","x\\y",şİı,"tab	in"}
ARRAY['a,b', '{x}', 'x}', 'null']::text[] -- array => {"a,b","{x}","x}","null"}
'[0:1][2:3]={{"a b",NULL},{"\\\\",""}}'::text[] -- array => [0:1][2:3]={{"a b",NULL},{"\\\\",""}}
ARRAY[true, false, NULL] -- array => {t,f,NULL}
ARRAY[-2]::int2[] -- array => {-2}
ARRAY[9223372036854775807]::int8[] -- array => {9223372036854775807}
ARRAY[4294967295]::oid[] -- array => {4294967295}
ARRAY[0.1, 'NaN', '-Infinity', 1e15, 1e-5]::float8[] -- array => {0.1,NaN,-Infinity,1e+15,1e-05}
ARRAY[0.1, 1234567, 'Infinity']::float4[] -- array => {0.1,1.234567e+06,Infinity}
ARRAY[12345.678, 'NaN', -1.50, 0.00000]::numeric[] -- array => {12345.678,NaN,-1.50,0.00000}
ARRAY['ab']::char(3)[] -- array => {"ab "}
ARRAY['abc']::varchar(10)[] -- array => {abc}
ARRAY['pg_class']::name[] -- array => {pg_class}
ARRAY['r']::"char"[] -- array => {r}
ARRAY[''::"char"] -- array => {""}
ARRAY['\x00ff0a', '\x']::bytea[] -- array => {"\\x00ff0a","\\x"}
ARRAY['6f1e9a0c-0000-4000-8000-000000000001']::uuid[] -- array => {6f1e9a0c-0000-4000-8000-000000000001}
ARRAY['{"a":1}', 'null']::json[] -- array => {"{\"a\":1}","null"}
ARRAY['{"b": 2, "a": [1, 2]}', '1']::jsonb[] -- array => {"{\"a\": [1, 2], \"b\": 2}",1}
ARRAY['2000-01-01', 'infinity', '4713-01-01 BC']::date[] -- array => {2000-01-01,infinity,"4713-01-01 BC"}
ARRAY['24:00:00', '00:00:01.5']::time[] -- array => {24:00:00,00:00:01.5}
ARRAY['01:00:00+03', '23:59:59-05']::timetz[] -- array => {01:00:00+03,23:59:59-05}
ARRAY['1999-12-31 23:59:59.999999', '-infinity']::timestamp[] -- array => {"1999-12-31 23:59:59.999999",-infinity}
ARRAY['2026-11-11 10:00:00+03', 'infinity']::timestamptz[] -- array => {"2026-11-11 07:00:00+00",infinity}
ARRAY['1 year 2 mons 3 days 04:05:06.000007', '0', '-1 days']::interval[] -- array => {"1 year 2 mons 3 days 04:05:06.000007",00:00:00,"-1 days"}
ARRAY['happy', 'sad']::pg_temp.hv_t07_mood[] -- array => {happy,sad}
ARRAY['192.168.0.1/24', '::1', '::ffff:1.2.3.4']::inet[] -- array => {192.168.0.1/24,::1,::ffff:1.2.3.4}
ARRAY['10.0.0.0/8']::cidr[] -- array => {10.0.0.0/8}
ARRAY['08:00:2b:01:02:03']::macaddr[] -- array => {08:00:2b:01:02:03}
ARRAY[B'101', B'']::varbit[] -- array => {101,""}
--
-- Network types and bit strings (binary).
'10.0.0.1'::inet -- network => 10.0.0.1
'10.0.0.0/8'::cidr -- network => 10.0.0.0/8
'192.168.1.7/32'::cidr -- network => 192.168.1.7/32
'::1'::inet -- network => ::1
'::'::inet -- network => ::
'2001:db8::1/64'::inet -- network => 2001:db8::1/64
'2001:db8::/32'::cidr -- network => 2001:db8::/32
'1:0:0:1:0:0:1:1'::inet -- network => 1::1:0:0:1:1
'1:0:1:1:1:1:1:1'::inet -- network => 1:0:1:1:1:1:1:1
'::ffff:1.2.3.4'::inet -- network => ::ffff:1.2.3.4
'::1.2.3.4'::inet -- network => ::1.2.3.4
'1::'::inet -- network => 1::
'::ffff:0:0'::inet -- network => ::ffff:0.0.0.0
'2001:db8::1/128'::cidr -- network => 2001:db8::1/128
'fe80::abcd:0:0:10/10'::inet -- network => fe80::abcd:0:0:10/10
'08:00:2b:01:02:03'::macaddr -- network => 08:00:2b:01:02:03
'08:00:2b:01:02:03:04:05'::macaddr8 -- network => 08:00:2b:01:02:03:04:05
B'10110'::varbit -- bit => 10110
B'101'::bit(3) -- bit => 101
B'111111110101'::varbit(20) -- bit => 111111110101
--
-- Text format, typed: money (lc_monetary C), xml, geometry, ranges, composites.
12.34::money -- money => $12.34
(-0.01)::money -- money => -$0.01
1234567.89::money -- money => $1,234,567.89
'<a>1</a>'::xml -- xml => <a>1</a>
'(1,2)'::point -- geometry => (1,2)
'((0,0),(1,1))'::box -- geometry => (1,1),(0,0)
'<(0,0),1>'::circle -- geometry => <(0,0),1>
'[1,3)'::int4range -- range => [1,3)
'(1,5]'::int4range -- range => [2,6)
'{[1,3), [5,7)}'::int4multirange -- range => {[1,3),[5,7)}
ROW(1, 'a b') -- composite => (1,"a b")
ROW(1, 'x')::pg_temp.hv_t07_pair -- composite => (1,x)
ARRAY['$1.00']::money[] -- array => {$1.00}
ARRAY[ROW(1, 'a')] -- array => {"(1,a)"}
--
-- Text format, opaque: no logical type of their own.
'a b'::tsvector -- opaque => 'a' 'b'
'1 2'::int2vector -- opaque => 1 2
'pg_class'::regclass -- opaque => pg_class
