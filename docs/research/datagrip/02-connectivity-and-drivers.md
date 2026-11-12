# DataGrip 2024.3.5 — connectivity, drivers, authentication, tunnels

> Covers: DBMS/driver inventory, URL templates and their grammar, default driver properties, driver artifacts and version constraints, authentication providers per DBMS, the connect interceptor pipeline, SSH/SSL/proxy/Kubernetes, cloud auth (AWS/Azure/GCP), Kerberos, pgpass, password expiry, test connection, error classification and quick fixes.
> Conventions and legal method: `00-method-and-legal.md`. Architecture context: `01-architecture-overview.md`. Session/transaction behaviour after the connection exists: `03-sessions-transactions-execution.md`.

---

## 1. Scope and sources

| Source | Version | Licence |
|---|---|---|
| `database-plugin.jar!/databaseDrivers/*.xml` (52 files, 78 `<driver>` entries, 115 `<url-template>` entries of which 107 non-empty) | DataGrip 2024.3.5 (243.24978.79) | Proprietary; URL formats reproduced as factual interface data |
| `database-plugin.jar!/resources/database-artifacts.xml` (64 artifacts) | same | Proprietary |
| Plugin descriptors and dialect XML (`connectionInterceptor`, `configValidator`, `optionProvider`, `urlParamEditorProvider` registrations) | same | Proprietary |
| `messages/DatabaseBundle.properties` (3,072 lines), platform `messages/SshBundle.properties` | same | Proprietary |
| Shipped help: `data-sources-and-drivers-dialog`, `configuring-database-connections`, `configuring-ssh-and-ssl`, `settings-tools-ssh-configurations`, `connectivity-problems`, per-DBMS pages (`postgresql`, `mysql`, `microsoft-sql-server`, `azure-sql-database`, `oracle`, `amazon-redshift`, `mongodb`, `redis`, `dynamodb`, `apache-cassandra`, `how-to-connect-to-cassandra-with-ssl`, `couchbase-query`, `clickhouse`, `sqlite`, `other-databases`, `managing-data-sources`, `run-a-query`) | 2024.3.5 offline help (pages dated 2024) | Proprietary docs; online at `https://www.jetbrains.com/help/datagrip/<topic>.html` |
| `DataGrip/redis-jdbc-driver@v1.5`, `DataGrip/mongo-jdbc-driver@v1.20` | GitHub | Apache-2.0 |
| Class-name inventory | 2024.3.5 | names only |

---

## 2. Executive summary

1. A **driver** is a declarative record: id, display name, dialect, driver class, `based-on` parent, URL templates, artifacts (jar sets with versions), default driver properties, options (`auto-sync`, `schema-control`, `send-app-info`, `auto-commit`), optional keep-alive query, server-version **constraints**, and **connection-bounds** (how to re-target the same server at another database) `[Observed: databaseDrivers/*.xml]`.
2. **URL templates are bidirectional**: the same template renders form fields into a URL *and* parses a pasted URL back into fields; several templates per driver are tried in order; each template *name* is a "connection type" in the UI (`default`, `Unix Socket`, `SID`, `Service Name`, `TNS`, `Long`, `cluster`, `MongoDB Atlas (SRV protocol)`, `IAM cluster/region`, `In-memory`, `Embedded`, …) plus the implicit `URL only` `[Observed: url-template name=; help:data-sources-and-drivers-dialog]`.
3. The template language has placeholders `{name:type/regex:default:label:escape}`, optional groups `[...]`, optional-token suffix `?`, repetition lists `<SEP,alt1,alt2,…>`, negative look-ahead `!(…)`, alternation `a|b`, and backslash escapes. A nameless `{:identifier}={:param}` element captures arbitrary extra driver properties (ALG-URL-01..03).
4. **Default properties encode hard-won fixes**: e.g. MySQL `tinyInt1isBit=false`, `zeroDateTimeBehavior=CONVERT_TO_NULL`, `yearIsDateType=false`; SQL Server `sendTimeAsDatetime=false`, `encrypt=false` (on-prem) vs `encrypt=true`/`trustServerCertificate=false` (Azure); Oracle `auto-commit=false`, `v$session.program=DataGrip`; ClickHouse `databaseTerm=schema`; SQLite `enable_load_extension=true`; HSQLDB `shutdown=true`; MongoDB `appName=DataGrip`; DocumentDB `retryWrites=false` (§3.4).
5. **Drivers are downloaded on demand** from Maven Central/JetBrains CDN, side-by-side versions, with "rolling" (auto-latest) or pinned versions, release channels, license files, Windows native auth DLLs, and **server-version constraints** that pick a different driver for old servers (MySQL < 5.2, Oracle < 11.2, MongoDB < 4.0, Db2 < 11) `[Observed: database-artifacts.xml, constraints]`.
6. **Connect pipeline** = ordered interceptors. First: password-expiry handling and PostgreSQL pgpass. Oracle TNS-names resolution runs *before* SSH. Then SSH tunnel, SSL, cloud-SQL proxies, expert options, logging, desktop/browser actions (SSO), forced-DBMS, per-DBMS auth providers, default credentials, and last `NoCredentialsAuthProvider` `[Observed: order= attributes]`.
7. **Auth methods** are per DBMS and pluggable: User & Password, No auth, pgpass (PG family), AWS Profile (Redshift, Athena, DynamoDB), Kerberos (SQL Server, Azure, Oracle, Mongo GSSAPI), Windows domain credentials / SSO (SQL Server), Azure AD password / interactive / integrated, Mongo SCRAM-SHA-1/256, AWS IAM, X.509, PLAIN(LDAP), BigQuery (service account, Google user account, tokens, ADC, identity federation), Snowflake external browser, Cloud Spanner credentials file (§3.8).
8. **SSH**: global or project-scoped SSH configurations referenced by id; auth = password, key pair (OpenSSH/PuTTY), or "OpenSSH config and authentication agent"; optional parsing of `~/.ssh/config`; keep-alive interval; strict host-key policy (Yes/Accept New/No/Ask); hashed known_hosts; HTTP/SOCKS proxy for the SSH hop; tunnels are **shared** between data sources; **one forward per host in the URL** (clusters) unless a fixed local port is forced, which then makes multi-host impossible `[Observed: help, SshBundle, bundle:notification.content.unable.to.create.multiple.ssh.tunnels]`.
9. **SSL**: CA file, client cert, client key (+ key password), truststore choice (IDE / Java / System), modes Require / Verify CA / Full Verification, "copy from another data source" `[Observed: help:configuring-ssh-and-ssl]`.
10. **Errors are classified** in the IDE by a base handler (credentials, driver class missing/incompatible, native library missing, IO, OOM of the driver process, SSL protocol, password expired / about to expire, SSH failure, process exited, unsafe update rolled back, …) plus per-DBMS handlers with targeted quick fixes (MySQL server time zone → set `serverTimezone`; MySQL TLS protocol list; Redis connection-mode mismatch; Oracle NLS/OCI; PgBouncer without prepared statements; SQLite file locked under WSL) (§3.13).

---

## 3. Detailed findings

### 3.1 Driver record vocabulary

`[Observed: union of all child tags/attributes in databaseDrivers/*.xml]`

| Element / attribute | Meaning | Examples |
|---|---|---|
| `driver@id` | Stable driver id (referenced by data sources as `driver-ref`) | `postgresql`, `mysql.8`, `sqlserver.ms`, `redis` |
| `@name` | UI name | "MySQL for 5.1" |
| `@dialect` | SQL dialect id for the editor | `PostgreSQL`, `TSQL`, `MongoJS`, `Redis`, `GenericSQL` |
| `@driver-class` / `@default-driver-class` | JDBC class; "default" when the artifact may change it | ClickHouse old `ru.yandex…` vs new `com.clickhouse…` decided by artifact property `driver.class` |
| `@based-on` | Inheritance from another driver (templates/properties/options merged; an empty `<url-template/>` in the child resets inherited templates) `[Inferred: children start with an empty url-template before redefining]` | `cockroach` based-on `postgresql`; `mysql.8` based-on `mysql.base` |
| `@abstract="true"` | Not user-selectable base | `mysql.base`, `oracle.base`, `mongo.base`, `db2.base` |
| `@group-with` | UI grouping of alternative drivers | `postgresql.aws_wrapper` grouped with `postgresql` |
| `url-template@name/@template` | Connection type + template | see §3.3 |
| `artifact@name/@use/@rolling/@channel` | Which artifact (jar set) to use; `use=true` = use by default; `rolling` = follow latest; `channel` = release line | `MariaDB Connector J` channel `MariaDB_2.X` for Aurora |
| `option@name/@value` | Data-source defaults | `auto-sync=true`, `schema-control=MANUAL`, `send-app-info=false`, `auto-commit=false` |
| `property@name/@value` | Default JDBC properties (Advanced tab) | `ApplicationName`, `tinyInt1isBit` |
| `keep-alive-query` | Driver-specific keep-alive SQL | Tibero: a `select … from dual` |
| `constraints/constraint@dbms/@from/@to` | Server version range this driver is valid for | `mysql` (5.1 driver) `to=5.2`; `oracle.19` `from=11.2` |
| `connection-bounds@default-value/@in-read-only/@bound-to` + `rewrite-parameter@name/@in-url` | How to point a connection at a different database on the same server (URL path segment and/or property) | PG: rewrite `database` in URL and `PGDBNAME`; default `postgres`. MySQL: `bound-to="schema"`, `in-read-only="true"` |
| `jdbc-additional-properties/property` | IDE-side flags | `forceUseDriverIcon=true` (DocumentDB, Databricks) |

Note: the PostgreSQL and CockroachDB files spell the attribute `defalt-value` (typo) while Azure/Synapse use `default-value` `[Observed]`. A Hacivat schema should be strict and versioned instead.

### 3.2 Complete driver inventory

`[Observed: databaseDrivers/*.xml]`. "Props" = default driver properties; "Opts" = data-source option defaults. Ports are the URL template defaults.

| Driver id | UI name | Dialect | Driver class | Based on | Default port | Props / Opts / notes |
|---|---|---|---|---|---|---|
| `postgresql` | PostgreSQL | PostgreSQL | `org.postgresql.Driver` | — | 5432 | Props `ApplicationName=` (filled by IDE); Opts `auto-sync`, `schema-control=MANUAL`; bounds rewrite `database` + `PGDBNAME`, default db `postgres` |
| `postgresql.cloud_sql` | PostgreSQL (CloudSQL Proxy) | (PG) | (PG) | postgresql | 5432 | `cloudSqlInstance=project:region:instance`, `socketFactory`; Unix-socket variant |
| `postgresql.aws_wrapper` | PostgreSQL (AWS Wrapper) | PostgreSQL | `software.amazon.jdbc.Driver` | — | 5432 | `useSSL=FALSE`; grouped with postgresql |
| `cockroach` | CockroachDB | Cockroach | (PG driver) | postgresql | 26257 | default db `defaultdb` |
| `greenplum` | Greenplum | Greenplum | (PG driver) | postgresql | 5432 | `send-app-info=false` |
| `redshift` | Amazon Redshift | Redshift | `com.amazon.redshift.jdbc.Driver` | — | 5439 | `enableFetchRingBuffer=false`; `schema-control=MANUAL`; IAM templates |
| `yugabyte` | YugabyteDB | PostgreSQL | `com.yugabyte.Driver` | — | 5433 | — |
| `mysql.base` (abstract) | — | MySQL | `com.mysql.cj.jdbc.Driver` | — | 3306 | `tinyInt1isBit=false`, `characterEncoding=utf8`, `characterSetResults=utf8`, `yearIsDateType=false`; bounds `bound-to=schema`, `in-read-only=true`, rewrite `DBNAME` + URL `database` |
| `mysql.8` | MySQL | (MySQL) | (cj) | mysql.base | 3306 | `zeroDateTimeBehavior=CONVERT_TO_NULL`; constraint MySQL ≥ 5.2 |
| `mysql` | MySQL for 5.1 | (MySQL) | `com.mysql.jdbc.Driver` | mysql.base | 3306 | `zeroDateTimeBehavior=convertToNull`; constraint MySQL < 5.2 |
| `mysql.cloud_sql.8` | MySQL (CloudSQL Proxy) | | | mysql.8 | — | socket factory |
| `mariadb` | MariaDB | MariaDB | `org.mariadb.jdbc.Driver` | — | 3306 | as MySQL + `zeroDateTimeBehavior=convertToNull`, `usePipelineAuth=false` |
| `mariadb.cloud_sql`, `mariadb.aws_wrapper` | | | | | 3306 | as above |
| `mysql_aurora`, `mysql_aurora_aws`, `mysql_aurora.aws_wrapper` | Aurora MySQL (MariaDB driver / AWS driver / AWS wrapper) | MySQL | various | | 3306 | `useSSL=FALSE` on AWS ones; MariaDB 2.X channel |
| `memsql`, `memsql_singlestore` | SingleStore (MySQL driver) / SingleStore | MySQL | `com.singlestore.jdbc.Driver` | mysql.8 / — | 3306 | `DatabaseMetaData=true` |
| `tidb` | TiDB | (MySQL) | | mysql.8 | 3306 | — |
| `sqlserver.ms` | Microsoft SQL Server | TSQL | `com.microsoft.sqlserver.jdbc.SQLServerDriver` | — | (SSRP-resolved; 1433) | `applicationName=DataGrip`, `sendTimeAsDatetime=false`, `encrypt=false`; `send-app-info=false` |
| `sqlserver.ms.cloud_sql` | SQL Server (CloudSQL Proxy) | | | sqlserver.ms | | `socketFactoryClass`, `socketFactoryConstructorArg` |
| `sqlserver.jtds` | Microsoft SQL Server (jTds) | TSQL | `net.sourceforge.jtds.jdbc.Driver` | — | | `appName=DataGrip` |
| `sqlserver.localdb.jtds` | Microsoft SQL Server LocalDB | | | sqlserver.jtds | (named pipe) | host fixed to `.`; `instance` from LocalDB pipe |
| `azure.ms` | Azure SQL Database | AZURE | (MS) | sqlserver.ms | 1433 | `loginTimeout=30`, `trustServerCertificate=false`, `encrypt=true`; bounds default db `master` |
| `synapse.ms` | Azure Synapse Analytics | AZURE | (MS) | sqlserver.ms | 1433 | + `hostNameInCertificate=*.sql.azuresynapse.net` |
| `sybase.jtds`, `sybase.ase` | Sybase (jTds) / Sybase | Sybase | jTDS / `com.sybase.jdbc4.jdbc.SybDriver` | — | 5000 | `appName`/`APPLICATIONNAME=DataGrip` |
| `oracle.base` (abstract) | — | Oracle | `oracle.jdbc.OracleDriver` | — | 1521 | `auto-commit=false` (opt); `v$session.program=DataGrip` |
| `oracle.19` | Oracle | | | oracle.base | 1521 | constraint ≥ 11.2 |
| `oracle` | Oracle for 9.X, 10.X, 11.1 | | | oracle.base | 1521 | constraint < 11.2; `oracle.jdbc.timezoneAsRegion=false` |
| `db2.base` (abstract), `db2.11`, `db2` | IBM Db2 / for 9.X,10.X | DB2 | `com.ibm.db2.jcc.DB2Driver` | | 50000 | `retrieveMessagesFromServerOnGetMessage=true`, `clientProgramName=DataGrip`; `auto-sync=false`; constraint LUW ≥/< 11.0 |
| `db2.jtopen` | IBM Db2 (JTOpen) | DB2 | `com.ibm.as400.access.AS400JDBCDriver` | | 446 | `prompt=false`; `auto-sync=false` |
| `sqlite.xerial` | SQLite | SQLite | `org.sqlite.JDBC` | | file | `enable_load_extension=true` |
| `h2.unified` | H2 | H2 | `org.h2.Driver` | | 9092 / mem / file | `MV_STORE` (advanced) |
| `hsqldb.local`, `hsqldb.remote` | HSQLDB | HSQLDB | `org.hsqldb.jdbc.JDBCDriver` / `org.hsqldb.jdbcDriver` | | 9001 | `shutdown=true` (local) |
| `derby.embedded`, `derby.remote` | Apache Derby | Derby | Embedded/Client | | 1527 | `create` checkbox |
| `clickhouse` | ClickHouse | ClickHouse | default `ru.yandex.clickhouse.ClickHouseDriver` (artifact may switch to `com.clickhouse.jdbc.ClickHouseDriver`) | | 8123 (HTTP) | `databaseTerm=schema` |
| `snowflake` | Snowflake | Snowflake | `net.snowflake.client.jdbc.SnowflakeDriver` | | 443 | artifacts Snowflake + `jna-platform` |
| `vertica` | Vertica | Vertica | `com.vertica.jdbc.Driver` | | 5433 | — |
| `exasol` | Exasol | Exasol | `com.exasol.jdbc.EXADriver` | | — | `clientname=DataGrip` |
| `hive` / `spark` / `databricks` | Hive / Spark / Databricks | HiveQL / SparkSQL | Hive driver / Databricks driver | | 10000 / 443 | `database` labelled "schema" |
| `bigquery` | Google BigQuery | BigQuery | `com.simba.googlebigquery.jdbc42.Driver` | | 443 | `MaxResults=10000`, `MetaDataFetchThreadCount=32` |
| `cloudspanner` | Google Cloud Spanner | GenericSQL | `com.google.cloud.spanner.jdbc.JdbcDriver` | | (9200 emulator) | — |
| `athena` | Athena | GenericSQL | `com.simba.athena.jdbc42.Driver` | | — | `MetadataRetrievalMethod=ProxyAPI` |
| `cassandra` | Apache Cassandra | CassandraQL | `com.dbschema.CassandraJdbcDriver` | | 9042 | `loadBalancing=DcInferringLoadBalancingPolicy` |
| `mongo_documentdb.base` (abstract), `mongo.base` (abstract), `mongo.4`, `mongo`, `documentdb` | MongoDB / MongoDB for 3.X / DocumentDB | MongoJS | `com.dbschema.MongoJdbcDriver` | | 27017 | `appName=DataGrip`; DocumentDB `retryWrites=false`; constraints Mongo ≥/< 4.0 |
| `redis` | Redis | Redis | `jdbc.RedisDriver` | | 6379 | templates `standalone`, `cluster` |
| `dynamo` | DynamoDB | Dynamo | `org.jetbrains.DynamoDriver` | | (AWS endpoint) | `region` default `eu-west-1` |
| `couchbase_query` | Couchbase Query | CouchbaseQuery | `com.intellij.CouchbaseJdbcDriver` | | (11210 per docs) | — |
| `elasticsearch` | Elasticsearch | GenericSQL | `org.elasticsearch.xpack.sql.jdbc.EsDriver` | | 9200 | X-Pack SQL JDBC |
| `duckdb` | DuckDB | GenericSQL | `org.duckdb.DuckDBDriver` | | file / in-memory | — |
| `trino`, `presto` | Trino / Presto | GenericSQL | `io.trino.jdbc.TrinoDriver` / `com.facebook.presto.jdbc.PrestoDriver` | | 8080 | — |
| `firebird`, `hana`, `informix`, `intersystemsiris`, `ignite`, `mimiersql`, `openedge`, `phoenix.thick`, `phoenix.thin`, `tarantool`, `teradata`, `tibero`, `denodo`, `odbc.sun` | (various) | GenericSQL | vendor classes | | 3050, —, 1526, 9091, —, 1360, -1, —, 8765, 3301, —, 8629, 9999 | Teradata `MetadataRetrievalMethod=ProxyAPI`; Tibero custom keep-alive query |

### 3.3 URL templates — factual table

`[Observed: databaseDrivers/*.xml url-template]`. Column "Type" = template name = the *Connection type* list in the General tab ("—" = unnamed). Templates are tried top-to-bottom when parsing. `\|` is a table escape of a literal `|`.

| Driver | Type | Template |
|---|---|---|
| `athena` | default | `jdbc:awsathena://<;,AwsRegion={region:database},User={user},Password={password},{:identifier}={:identifier/[^;]*}>;?` |
| `azure.ms` | default | `jdbc:sqlserver://{host:host_ipv6:server.database.windows.net}[:{port::1433}][;<;,user[Name]={user:param},password={password:param},database[Name]={database},{:identifier}={:param}>];?` |
| `azure.ms` | default | `jdbc:sqlserver://;<;,user[Name]={user},password={password},database[Name]={database},server[Name]={host:host_ipv6:server.database.windows.net},port[Number]={port::1433},{:identifier}={:param}>;?` |
| `bigquery` | default | `jdbc:bigquery://https://{host:host/[^\:;]*:www.googleapis.com/bigquery/v2}[:[{port::443}]][;<;,ProjectId={ProjectId:#param:},OAuthType={OAuthType:#param:},OAuthAccessToken={OAuthAccessToken:#param:},OAuthRefreshToken={OAuthRefreshToken:#param:},OAuthClientId={OAuthClientId:#param:},OAuthClientSecret={OAuthClientSecret:#param:},OAuthServiceAcctEmail={OAuthServiceAcctEmail:#param:};OAuthPvtKeyPath={OAuthPvtKeyPath:#param:},DefaultDataset={DefaultDataset:param},{:identifier}={:param}>];?` |
| `cassandra` | default | `jdbc:cassandra://<\,,{host::localhost}?[:{port::9042}]>[/{database:::keyspace}?[\?<&,user={user},password={password},{:identifier}={:identifier}>]]` |
| `clickhouse` | default | `jdbc:clickhouse://{host::localhost}?[:{port::8123}][/{database}?][\?<&,user={user},password={password},{:identifier}={:identifier}>]` |
| `clickhouse` | default | `jdbc:clickhouse://\[{host:ipv6:\:\:1}\][:{port::8123}][/{database}?][\?<&,user={user},password={password},{:identifier}={:identifier}>]` |
| `cloudspanner` | default | `jdbc:cloudspanner:/projects/{project:identifier/[^/]+}/instances/{instance:identifier/[^/]+}/databases/{database}[;<;,{:identifier/[^&=]+}={:identifier/[^;]*}>;?]` |
| `cloudspanner` | With host | `jdbc:cloudspanner://{host::localhost}:{port::9200}/projects/{project:identifier/[^/]+}/instances/{instance:identifier/[^/]+}/databases/{database}[;<;,{:identifier/[^&=]+}={:identifier/[^;]*}>;?]` |
| `cockroach` | default | `jdbc:postgresql://[{host::localhost}[:{port::26257}]]/[{database:database/[^?]+:defaultdb::%}?][\?<&,user={user:param},password={password:param},{:identifier}={:param}>]` |
| `cockroach` | default | `jdbc:postgresql://\[{host:ipv6:\:\:1}\][:{port::26257}]/[{database:database/[^?]+:defaultdb::%}?][\?<&,user={user:param},password={password:param},{:identifier}={:param}>]` |
| `cockroach` | default | `jdbc:postgresql:{database:database/(?!//)[^?]+:defaultdb::%}[\?<&,user={user:param},password={password:param},{:identifier}={:param}>]` |
| `couchbase_query` | default | `jdbc:couchbase:<\,,{host::localhost}[:{port}]>[[/{default_bucket}?][\?<&,user={user},password={password},{:identifier}={:identifier}>]]` |
| `odbc.sun` | — | `jdbc:odbc:{database}` |
| `db2.base` | Type 4 | `jdbc:db2://{host::localhost}[:{port::50000}][/{database}][:<;,user={user:param},password={password:param},{:identifier}={:param}>;]` |
| `db2.base` | Type 4 | `jdbc:db2j:net://{host::localhost}[:{port::50000}][/{database}][:<;,user={user:param},password={password:param},{:identifier}={:param}>;]` |
| `db2.base` | Type 4 | `jdbc:ids://{host::localhost}[:{port::50000}][/{database}][:<;,user={user:param},password={password:param},{:identifier}={:param}>;]` |
| `db2.base` | Type 4 | `jdbc:informix-sqli://{host::localhost}[:{port::50000}][/{database}][:<;,user={user:param},password={password:param},{:identifier}={:param}>;]` |
| `db2.base` | Type 2 | `jdbc:db2:{database::database}[:<;,user={user},password={password},{:identifier}={:param}>;]` |
| `db2.jtopen` | default | `jdbc:as400://{host::localhost}[:{port::446}][/{database}][;<;,user={user:param},password={password:param},{:identifier}={:param}>;?]` |
| `denodo` | default | `jdbc:denodo://{host::localhost}:{port::9999}/{database}?[\?<&,user={user:param},password={password:param},{:identifier}={:param}>]` |
| `denodo` | default | `jdbc:vdb://{host::localhost}:{port::9999}/{database}?[\?<&,user={user:param},password={password:param},{:identifier}={:param}>]` |
| `derby.embedded` | default | `jdbc:derby:!(memory:){path}[;<;,create={create database:true_box},user={user:param},password={password:param},{:identifier}={:param}>]` |
| `derby.embedded` | In-Memory | `jdbc:derby:memory:{path}[;<;,create={create database:true_box:true},user={user:param},password={password:param},{:identifier}={:param}>]` |
| `derby.remote` | default | `jdbc:derby://{host::localhost}:{port::1527}/{database:path}[;<;,user={user:param},password={password:param},{:identifier}={:param}>]` |
| `duckdb` | default | `jdbc:duckdb:{file::identifier.db}` |
| `duckdb` | In-memory | `jdbc:duckdb:` |
| `dynamo` | default | `jdbc:dynamodb://[{host::}[:{port::}]][\?<&,region={region:param:eu-west-1},{:identifier}={:param}>]` |
| `elasticsearch` | default | `jdbc:es://[http://\|https://][{host::localhost}[:{port::9200}]]/{prefix:param/[^?]*}[?<&,user={user},password={password},{:identifier/[^&=]+}={:identifier/[^&]*}>]` |
| `elasticsearch` | default | `jdbc:elasticsearch://[http://\|https://][{host::localhost}[:{port::9200}]]/{prefix:param/[^?]*}[?<&,user={user},password={password},{:identifier/[^&=]+}={:identifier/[^&]*}>]` |
| `exasol` | — | `jdbc:exa:{host::localhost}:{port}[;<;,user={user:param},password={password:param},{:identifier}={:param}>]` |
| `firebird` | default | `jdbc:firebirdsql://{host::localhost}?[:{port::3050}][/{database:path}?][\?<&,user={user},password={password},{:identifier}={:identifier}>]` |
| `h2.unified` | Remote | `jdbc:h2:tcp://{host::localhost}[:{port::9092}]/{database::default}[;<;,user={user:param},password={password:param},{:identifier}={:param}>]` |
| `h2.unified` | In-memory | `jdbc:h2:mem:{database::default}?[;<;,{:identifier}={:param}>]` |
| `h2.unified` | Embedded | `jdbc:h2:!(mem:)!(tcp://)[file:]{path:h2_db_file}[;<;,user={user:param},password={password:param},MV_STORE={MV_STORE:#param},{:identifier}={:param}>]` |
| `hana` | default | `jdbc:sap://<;,{host::localhost}[:{port}]>[/\?<&,databaseName={database},user={user},password={password},{:identifier}={:identifier/[^,]*}>]` |
| `hive` | default | `jdbc:hive2://{host::localhost}?[:{port::10000}][/{database:::schema}?][;<;,user={user},password={password},{:identifier}={:identifier}>]` |
| `hsqldb.local` | Embedded | `jdbc:hsqldb:file:[[{path:path_db}/]{database}][;<;,ifexists={if exists:true_box:true},user={user:param},password={password:param},{:identifier}={:param}>]` |
| `hsqldb.local` | In-memory | `jdbc:hsqldb:mem:{database::default}?[;<;,{:identifier}={:param}>]` |
| `hsqldb.remote` | default | `jdbc:hsqldb:hsql://{host::localhost}[:{port::9001}][/{database::default}?][;<;,user={user:param},password={password:param},{:identifier}={:param}>]` |
| `ignite` | default | `jdbc:ignite:thin://{host}[:{port}][/{database:::schema}?][\?<&,user={user},password={password},{:identifier}={:identifier}>]` |
| `ignite` | default | `jdbc:ignite:thin://{host}[:{port}][;<;,schema={database:::schema},user={user},password={password},{:identifier}={:identifier}>]` |
| `informix` | default | `jdbc:informix-sqli://{host::localhost}:{port::1526}/{database}[:<;,USER={user},PASSWORD={password},{:identifier}={:identifier/[^;]*}>;?]` |
| `informix` | direct | `jdbc:informix-direct://{database}[;<;,USER={user},PASSWORD={password},{:identifier}={:identifier/[^;]*}>;?]` |
| `intersystemsiris` | default | `jdbc:IRIS://{host::localhost}?[:{port::9091}][/{database}?][\?<&,user={user},password={password},{:identifier}={:identifier}>]` |
| `mariadb` | default | `jdbc:mariadb://{host::localhost}?[:{port::3306}][/{database}?][\?<&,user={user},password={password},{:identifier}={:param}>]` |
| `mariadb` | default | `jdbc:mariadb://address=\(protocol=tcp\)\(<\)\(,host={host:host_ipv6:localhost},port={port::3306},user={user},password={password},{:identifier}={:identifier}>\)[/{database}][\?<&,{:identifier}={:param}>]` |
| `mariadb` | Unix Socket | `jdbc:mariadb://[\?<&,localSocket={mysql.socket::/tmp/mysqld.sock},user={user},password={password},{:identifier}={:param}>]` |
| `mariadb.cloud_sql` | default | `jdbc:mariadb://{:host:localhost}:{:port:3306}[/{database}?][\?<&,user={user},password={password},cloudSqlInstance={project:identifier}:{region:identifier}:{instance:identifier},socketFactory={:param},{:identifier}={:param}>]` |
| `mariadb.aws_wrapper` | default | `jdbc:aws-wrapper:mariadb://{host::localhost}?[:{port::3306}][/{database}?][\?<&,user={user},password={password},{:identifier}={:param}>]` |
| `memsql_singlestore` | default | `jdbc:singlestore://{host::localhost}?[:{port::3306}][/{database}?][\?<&,user={user},password={password},{:identifier}={:param}>]` |
| `mimiersql` | default | `jdbc:mimer://{host::localhost}?[:{port::1360}]/{database}[\?<&,user={user},password={password},{:identifier}={:identifier}>]` |
| `mimiersql` | default | `jdbc:mimer://{user}[:{password}]@{host::localhost}?[:{port::1360}]/{database}[\?<&,{:identifier}={:identifier}>]` |
| `mongo_documentdb.base` | default | `[jdbc:]mongodb[+srv]://[{user:identifier}[:{password:param}]@]<\,,{host::localhost}?[:{port::27017}]>[/{database}?[\?<&,authSource={authSource:#param:},authMechanism={authMechanism:#param:},authMechanismProperties={authMechanismProperties:#param:},{:identifier}={:param}>]]` |
| `mongo.base` | MongoDB Atlas (SRV protocol) | `[jdbc:]mongodb+srv://[{user:identifier}[:{password:param}]@]<\,,{host::localhost}?>[/{database}?[\?<&,{:identifier}={:param}>]]` |
| `mysql_aurora.aws_wrapper` | default | `jdbc:aws-wrapper:mysql://{host::localhost}?[:{port::3306}][/{database}?][\?<&,user={user},password={password},{:identifier}={:param}>]` |
| `mysql_aurora_aws` | default | `jdbc:mysql:aws://{host::localhost}?[:{port::3306}][/{database}?][\?<&,user={user},password={password},{:identifier}={:param}>]` |
| `mysql_aurora` | default | `jdbc:mysql:aurora://{host::localhost}?[:{port::3306}][/{database}?][\?<&,user={user},password={password},{:identifier}={:param}>]` |
| `mysql_aurora` | default | `jdbc:mysql:aurora://address=\(protocol=tcp\)\(<\)\(,host={host:host_ipv6:localhost},port={port::3306},user={user},password={password},{:identifier}={:identifier}>\)[/{database}][\?<&,{:identifier}={:param}>]` |
| `mysql.base` | default | `jdbc:mysql://{host::localhost}?[:{port::3306}][/{database}?][\?<&,user={user},password={password},{:identifier}={:param}>]` |
| `mysql.base` | default | `jdbc:mysql://address=\(protocol=tcp\)\(<\)\(,host={host:host_ipv6:localhost},port={port::3306},user={user},password={password},{:identifier}={:param}>\)[/{database}][\?<&,{:identifier}={:param}>]` |
| `mysql.base` | Unix Socket | `jdbc:mysql:///{database}?[\?<&,user={user},password={password},junixsocket.file={mysql.socket::/tmp/mysqld.sock},socketFactory={socketFactory:#param:org.newsclub.net.mysql.AFUNIXDatabaseSocketFactory},{:identifier}={:param}>]` |
| `mysql.cloud_sql.8` | default | `jdbc:mysql://{:host}?[:{:port}][/{database}?][\?<&,user={user},password={password},cloudSqlInstance={project:identifier}:{region:identifier}:{instance:identifier},socketFactory={:param},{:identifier}={:param}>]` |
| `mysql.cloud_sql.8` | Unix Socket | `jdbc:mysql:///{database}?[\?<&,user={user},password={password},unixSocketPath={mysql.socket::/tmp/mysqld.sock},cloudSqlInstance={project:identifier}:{region:identifier}:{instance:identifier},socketFactory={:param},{:identifier}={:param}>]` |
| `openedge` | default | `jdbc:datadirect:openedge://{host::localhost}:{port:port/-?[0-9]+:-1}[;<;,servicename={service:identifier},databaseName={database},user={user},password={password},{:identifier}={:identifier/[^;]*}>];?` |
| `oracle.base` | SID | `jdbc:oracle:{driver:variants:thin:Thin=thin,OCI=oci,OCI8=oci8}:[{user}[/{password}]]@{host::localhost}?[:{port::1521}]:{SID:identifier:XE}` |
| `oracle.base` | SID | `jdbc:oracle:{driver:variants:thin:Thin=thin,OCI=oci,OCI8=oci8}:[{user}[/{password}]]@\[{host:ipv6:\:\:1}\][:{port::1521}]:{SID:identifier:XE}` |
| `oracle.base` | Service Name | `jdbc:oracle:{driver:variants:thin:Thin=thin,OCI=oci,OCI8=oci8}:[{user}[/{password}]]@//{host::localhost}?[:{port::1521}]/{service:identifier:XE}` |
| `oracle.base` | Service Name | `jdbc:oracle:{driver:variants:thin:Thin=thin,OCI=oci,OCI8=oci8}:[{user}[/{password}]]@//\[{host:ipv6:\:\:1}\][:{port::1521}]/{service:identifier:XE}` |
| `oracle.base` | TNS | `jdbc:oracle:{driver:variants:thin:Thin=thin,OCI=oci,OCI8=oci8}:[{user}[/{password}]]@{tns_name}[\?<&,{:identifier}={:param}>]` |
| `oracle.base` | Long | `jdbc:oracle:{driver:variants:thin:Thin=thin,OCI=oci,OCI8=oci8}:[{user}[/{password}]]@ \( description = \( < \) \( ,address = \( < \) \( ,!!protocol = tcp,host = {host::localhost},port = {port::1521},{:identifier} = {:ora_parens}> \),connect_data = \( < \) \( ,service_name = {service:identifier:XE},{:identifier} = {:ora_parens}> \),{:identifier} = {:ora_parens}> \) \) [\?<&,{:identifier}={:param}>]` |
| `phoenix.thick` | default | `jdbc:phoenix:!(thin:)<\,,{host::localhost}>[:{port}[:{node:identifier/[^\:]*}[:{principal:identifier/[^\:]*}[:{keytab:path}]]]]` |
| `phoenix.thin` | default | `jdbc:phoenix:thin:<;,url=https?://{host::localhost}[:{port::8765}],principal={principal:identifier/[^;]*},keytab={keytab:path},{:identifier}={:identifier/[^;]*}>;?` |
| `postgresql` | default | `jdbc:postgresql://[{host::localhost}[:{port::5432}]]/[{database:database/[^?]+:postgres::%}?][\?<&,user={user:param},password={password:param},{:identifier}={:param}>]` |
| `postgresql` | default | `jdbc:postgresql://\[{host:ipv6:\:\:1}\][:{port::5432}]/[{database:database/[^?]+:postgres::%}?][\?<&,user={user:param},password={password:param},{:identifier}={:param}>]` |
| `postgresql` | default | `jdbc:postgresql:{database:database/(?!//)[^?]+:postgres::%}[\?<&,user={user:param},password={password:param},{:identifier}={:param}>]` |
| `postgresql.cloud_sql` | default | `jdbc:postgresql://[{:host:localhost}[:{:port:5432}]]/[{database:database/[^?]+:postgres::%}?][\?<&,user={user},password={password},cloudSqlInstance={project:identifier}:{region:identifier}:{instance:identifier},socketFactory={:param},{:identifier}={:param}>]` |
| `postgresql.cloud_sql` | Unix Socket | `jdbc:postgresql:///[{database:database/[^?]+:postgres::%}?][\?<&,user={user},password={password},unixSocketPath={mysql.socket::/tmp/mysqld.sock},cloudSqlInstance={project:identifier}:{region:identifier}:{instance:identifier},socketFactory={:param},{:identifier}={:param}>]` |
| `postgresql.aws_wrapper` | default | `jdbc:aws-wrapper:postgresql://[{host::localhost}[:{port::5432}]]/[{database:database/[^?]+:postgres::%}?][\?<&,user={user:param},password={password:param},{:identifier}={:param}>]` |
| `presto` | default | `jdbc:presto://{host::localhost}?[:{port::8080}][/{database:path}?][\?<&,user={user},password={password},{:identifier}={:identifier}>]` |
| `redis` | standalone | `jdbc:redis://[[{user}:]{password}@]{host::localhost}[:{port::6379}][/{database:database/[^?]+:0}?][\?<&,{:identifier}={:param}>]` |
| `redis` | cluster | `jdbc:redis:cluster://[[{user}:]{password}@]<\,,{host::localhost}[:{port::6379}]>[/{database:database/[^?]+:0}?][\?<&,{:identifier}={:param}>]` |
| `redshift` | default | `jdbc:redshift:{iam:#variants::IAM=iam\:,NONE=}//[{host::server.redshift.amazonaws.com}[:{port::5439}]][/{database::::%}?][\?<&,user={user:param},password={password:param},{:identifier}={:param}>]` |
| `redshift` | default | `jdbc:postgresql://[{host::server.redshift.amazonaws.com}[:{port::5439}]][/{database::::%}?][\?<&,user={user:param},password={password:param},{:identifier}={:param}>]` |
| `redshift` | IAM cluster/region | `jdbc:redshift:iam://{cluster:host:mycluster}:{region:host:eu-west-1}[/{database::::%}?][\?<&,user={user:param},password={password:param},{:identifier}={:param}>]` |
| `snowflake` | default | `jdbc:snowflake://{host}[:{port::443}]/?[\?<&,user={user:param:::%},password={password:param:::%},warehouse={warehouse:param:::%},db={database::::%},schema={schema:##param:::%},role={role:##param:::%},{:identifier:::%}={:param:::%}>]` |
| `spark` | default | `jdbc:hive2://{host::localhost}?[:{port::10000}][/{database:::schema}?][;<;,user={user},password={password},{:identifier}={:identifier}>]` |
| `databricks` | default | `jdbc:databricks://{host::localhost}[:{port::443}][/{database:::schema}?][;<;,user={user},password={password},{:identifier}={:param}>?]` |
| `sqlite.xerial` | default | `jdbc:sqlite:!(:memory:){file::identifier.sqlite}?` |
| `sqlite.xerial` | In-memory | `jdbc:sqlite::memory:` |
| `sqlserver.ms` | default | `jdbc:sqlserver://{host:ssrp_host:localhost}[\\{instance:ssrp_instance}][:{port:ssrp_port}][;<;,user[Name]={user:param},password={password:param},database[Name]={database},{:identifier}={:param}>];?` |
| `sqlserver.ms` | default | `jdbc:sqlserver://;<;,user[Name]={user},password={password},database[Name]={database},server[Name]={host:ssrp_host_ipv6:localhost}[\\{instance:ssrp_instance}],port[Number]={port:ssrp_port},{:identifier}={:param}>;?` |
| `sqlserver.ms` | default | `jdbc:sqlserver://{host:ssrp_host:localhost}[:{port:ssrp_port}][;<;,user[Name]={user},password={password},database[Name]={database},instanceName={instance:ssrp_instance},{:identifier}={:param}>];?` |
| `sqlserver.ms` | default | `jdbc:sqlserver://;<;,user[Name]={user},password={password},database[Name]={database},server[Name]={host:ssrp_host_ipv6:localhost},instanceName={instance:ssrp_instance},port[Number]={port:ssrp_port},{:identifier}={:param}>;?` |
| `sqlserver.ms.cloud_sql` | default | `jdbc:sqlserver://{:host:localhost}[\\{:database}][:{:port}][;<;,user[Name]={user:param},password={password:param},database[Name]={database},socketFactoryConstructorArg={project:identifier}:{region:identifier}:{instance:identifier},socketFactoryClass={:param},{:identifier}={:param}>]` |
| `sqlserver.jtds` | default | `jdbc:jtds:sqlserver://{host:ssrp_host:localhost::[}[:{port:ssrp_port}][/{database::::[}][;<;,instance={instance:ssrp_instance:::[},user={user:param:::[},password={password:param:::[},{:identifier}={:param:::[}>]` |
| `sqlserver.localdb.jtds` | default | `jdbc:jtds:sqlserver://{#host::.::[}/{database::::[}?[;<;,instance={localdb_pipe},user={user:param:::[},password={password:param:::[},{:identifier}={:param:::[}>]` |
| `sybase.jtds` | default | `jdbc:jtds:sybase://{host::localhost}[:{port::5000}][/{database}][;<;,user={user:param},password={password:param},{:identifier}={:param}>]` |
| `sybase.ase` | default | `jdbc:sybase:Tds:{host::localhost}:{port::5000}/{database::master}[\?<&,user={user:param},password={password:param},{:identifier}={:param}>&?]` |
| `synapse.ms` | default | `jdbc:sqlserver://{host:host_ipv6:server.sql.azuresynapse.net}[:{port::1433}][;<;,user[Name]={user:param},password={password:param},database[Name]={database},{:identifier}={:param}>];?` |
| `synapse.ms` | default | `jdbc:sqlserver://;<;,user[Name]={user},password={password},database[Name]={database},server[Name]={host:host_ipv6:server.sql.azuresynapse.net},port[Number]={port::1433},{:identifier}={:param}>;?` |
| `tarantool` | default | `jdbc:tarantool://{host::localhost}:{port::3301}[?<&,user={user},password={password},{:identifier}={:identifier/[^&]*}>]` |
| `teradata` | default | `jdbc:teradata://{host::localhost}[/<\,,DATABASE={database},USER={user},PASSWORD={password},{:identifier}={:identifier/[^,]*}>]` |
| `tibero` | default | `jdbc:tibero:thin:[{user}[/{password}]]@{host::localhost}:{port::8629}:{database}` |
| `trino` | default | `jdbc:trino://{host::localhost}?[:{port::8080}][/{database:path}?][\?<&,user={user},password={password},{:identifier}={:identifier}>]` |
| `vertica` | default | `jdbc:vertica://{host::localhost}?[:{port::5433}][/{database}?][\?<&,user={user},password={password},{:identifier}={:identifier}>]` |
| `yugabyte` | default | `jdbc:yugabytedb://{host::localhost}?[:{port::5433}][/{database}?][\?<&,user={user},password={password},{:identifier}={:param}>]` |

### 3.4 Template language (reconstructed from the corpus)

The help states only "`{…}` are variables, `[…]` optional fragments, template names = connection types" `[Observed: help:data-sources-and-drivers-dialog]`. The following grammar is `[Inferred]` from all 107 templates; each rule cites a witness.

| Construct | Meaning | Witness |
|---|---|---|
| literal text | must match exactly (case-sensitive except where the driver is lenient) | `jdbc:postgresql://` |
| `\x` | escape of a meta character: `\?`, `\[`, `\]`, `\(`, `\)`, `\:`, `\,`, `\\` | `\[{host:ipv6:\:\:1}\]` |
| `{name}` | named placeholder bound to a form field | `{tns_name}`, `{host}` |
| `{name:type}` | typed placeholder; *type* selects validator + editor widget | `host`, `host_ipv6`, `ipv6`, `port`, `database`, `param`, `identifier`, `path`, `true_box` (checkbox), `variants` (dropdown), `ssrp_host`/`ssrp_instance`/`ssrp_port`/`ssrp_host_ipv6` (SQL Server Browser instance discovery), `mysql.socket`, `h2_db_file`, `path_db`, `localdb_pipe`, `ora_parens` (balanced Oracle descriptor value) |
| `{name:type/regex}` | the value must match *regex* (and, when parsing, the regex bounds how much text is consumed) | `{database:database/[^?]+}`, `{project:identifier/[^/]+}` |
| `{name:type:default}` | default shown as placeholder and used when empty | `{port::5432}`, `{region:param:eu-west-1}` |
| 4th field | **UI label override** | `{database:::schema}` (Hive/Spark), `{database:::keyspace}` (Cassandra) |
| 5th field | **escaping mode** of the rendered value: `%` = percent-encode (URL-style), `[` = a jTDS-specific escaping mode `[Unknown exact rule]` | `{database:database/[^?]+:postgres::%}`, `{user:param:::[}` |
| `#` prefix on type (`#param`, `#variants`) | value not shown as a main-form field (advanced/hidden) `[Inferred]` | `{ProjectId:#param:}` (BigQuery), `{iam:#variants::IAM=iam\:,NONE=}` |
| `##` prefix | stronger visibility flag (e.g. rendered only when set) `[Unknown]` | Snowflake `schema`, `role` |
| `#` prefix on *name* | fixed/hidden field | `{#host::.::[}` LocalDB host `.` |
| `{:type…}` (no name) | anonymous capture: parsed but not bound to a named field; when rendering, its default is emitted | `{:host:localhost}` (Cloud SQL proxy: host is the proxy, not user-editable) |
| `{:identifier}={:param}` inside a list | **passthrough key/value**: unknown URL parameters are captured into the driver-properties table and rendered back | in every `<…>` list |
| `variants` default + mapping | `{driver:variants:thin:Thin=thin,OCI=oci,OCI8=oci8}` = dropdown, default `thin`, label=value pairs | Oracle |
| `[ … ]` | optional group: rendered only if at least one placeholder inside has a non-empty value (or a literal-only group is chosen when parsing) | `[:{port::5432}]` |
| `[a\|b]` | alternation inside an optional group | `[http://\|https://]` (Elasticsearch) |
| `x?` | the preceding token (placeholder, literal char) may be empty/absent | `{host::localhost}?`, `/{database}?`, trailing `;?`, `&?` |
| `user[Name]=` | optional literal inside a key → accepts both `user=` and `userName=` (renders the short form) | SQL Server |
| `<SEP,alt1,alt2,…>` | **repetition list**: zero or more items joined by `SEP`; each item matches one of the comma-separated alternatives; `SEP` is the text before the first unescaped comma and may be multi-char (`\)\(`) or an escaped comma (`\,`) | `<&,user={user},password={password},{:identifier}={:param}>`; host lists `<\,,{host::localhost}[:{port::9042}]>` |
| `!!item` inside a list | item always rendered (mandatory constant) `[Inferred]` | Oracle Long: `!!protocol = tcp` |
| `!(text)` | negative look-ahead: the input at this point must **not** start with *text* (disambiguates templates of one driver) | `jdbc:sqlite:!(:memory:)…`, `jdbc:h2:!(mem:)!(tcp://)…`, `jdbc:phoenix:!(thin:)…` |
| spaces around ` = ` and `\(` in Oracle Long | `[Inferred]` whitespace-tolerant matching | Oracle Long template |

Two template sets per DBMS commonly co-exist: a hostname form and a bracketed **IPv6** form (`\[{host:ipv6:\:\:1}\]`), plus a "properties-only" form (SQL Server `jdbc:sqlserver://;serverName=…`). PostgreSQL also accepts the host-less `jdbc:postgresql:dbname` form, guarded by the regex `(?!//)`.

### 3.5 Why the default properties matter

| Driver | Property / option | Effect and reason `[Inferred unless marked]` |
|---|---|---|
| PostgreSQL | `ApplicationName` (empty, IDE fills) | Shows the tool in `pg_stat_activity.application_name`; DBAs can identify/kill IDE sessions. Option `send-app-info` controls this globally `[Observed: help: "Send application info"]`. |
| PostgreSQL, Redshift | `schema-control=MANUAL` | The IDE does not issue `SET search_path` automatically by default; user controls it (see `03` schema switching). |
| Redshift | `enableFetchRingBuffer=false` | Disables the driver's background prefetch thread so the IDE's own paging/cancel semantics hold and memory stays bounded. |
| MySQL/MariaDB | `tinyInt1isBit=false` | Prevents `TINYINT(1)` from surfacing as boolean → grids show real values 0..255. |
| MySQL/MariaDB | `zeroDateTimeBehavior=CONVERT_TO_NULL` (`convertToNull` for 5.1/MariaDB) | `0000-00-00` dates would otherwise throw on read; shown as NULL instead. |
| MySQL/MariaDB | `yearIsDateType=false` | `YEAR` shown as short integer, not a date. |
| MySQL/MariaDB | `characterEncoding/characterSetResults=utf8` | Avoids server-default latin1 mojibake. |
| MariaDB | `usePipelineAuth=false` | Pipelined auth breaks with some proxies (ProxySQL, MaxScale) — conservative default. |
| SQL Server | `sendTimeAsDatetime=false` | Sends `java.sql.Time` as `time` not `datetime` (avoids 1900-01-01 artefacts and index mismatches). |
| SQL Server | `encrypt=false` (on-prem), `applicationName=DataGrip` | Newer MS drivers default `encrypt=true` and fail on self-signed certs; the IDE chooses compatibility for on-prem; Azure/Synapse flip to `encrypt=true`, `trustServerCertificate=false`, `loginTimeout=30` and set `hostNameInCertificate` for Synapse wildcard certs `[Observed values]`. |
| Oracle | `auto-commit=false` | Oracle culture: work in explicit transactions; consoles start in manual tx mode. |
| Oracle | `v$session.program=DataGrip` | Identifies the session in `V$SESSION.PROGRAM`. |
| Oracle (≤11.1 driver) | `oracle.jdbc.timezoneAsRegion=false` | Avoids `ORA-01882: timezone region not found` when the client JVM TZ is unknown to the server; the docs give the same advice `[Observed: help:data-sources-and-drivers-dialog]`. |
| Db2 | `retrieveMessagesFromServerOnGetMessage=true`, `clientProgramName` | Full error text instead of SQLCODE only; session identification. |
| JTOpen | `prompt=false` | Prevents the AS/400 driver from opening a native GUI sign-on prompt. |
| ClickHouse | `databaseTerm=schema` | Maps ClickHouse databases to JDBC *schemas* so the model is Root → Schema → Table. |
| SQLite | `enable_load_extension=true` | Allows `load_extension()` (spatialite etc.). |
| HSQLDB local | `shutdown=true` | Closes the embedded DB cleanly when the last connection closes (avoids lock files). |
| MongoDB | `appName=DataGrip` | Visible in `currentOp`/logs. |
| DocumentDB | `retryWrites=false` | DocumentDB rejects retryable writes. |
| Cassandra | `loadBalancing=DcInferringLoadBalancingPolicy` | Driver 4.x requires a local DC; infers it for single-DC/dev clusters. |
| BigQuery | `MaxResults=10000`, `MetaDataFetchThreadCount=32` | Page size and parallel metadata fetching (many datasets). |
| Athena, Teradata | `MetadataRetrievalMethod=ProxyAPI` | Uses catalog API instead of slow `SHOW`/query-based metadata. |
| AWS wrapper drivers | `useSSL=FALSE` | Wrapper delegates TLS to the target driver config. |

### 3.6 Driver artifacts, versions and constraints

`[Observed: resources/database-artifacts.xml]`
- 64 artifacts; each `<version>` has `<item>`s of type `maven` (GAV coordinate, resolved with the bundled Maven resolver), plain URL (JetBrains CDN `download.jetbrains.com/idea/jdbc-drivers/...`), `license`, `native` (OS-specific, e.g. Windows integrated-auth DLL jars for SQL Server/jTDS, `os="win"`), `pack`.
- Repositories can be declared (e.g. an `alibaba` Maven mirror for OceanBase).
- `<constraint from="X" dbms="D" db-from="V">` and `<unsupported from=… db-to=…>`: a driver version range is valid only for server versions in a range (e.g. ClickHouse driver ≥ 0.3.2 requires server ≥ 20.7; MongoDB driver ≥ 1.19 requires Mongo ≥ 4; Oracle driver 12+ ↔ server ≥ 11.2; Db2 4.26 ↔ LUW ≥ 11.0).
- `<unsupported ide-to="222.3245"/>`: versions hidden from older IDE builds.
- An artifact can switch the driver class per version (`<property name="driver.class">`), e.g. ClickHouse legacy vs new driver.
- Channels: `MariaDB_2.X` "MariaDB Connector/J 2.X" with `implicit="true"` (used by Aurora MySQL template).
- Downloaded jars live in the IDE config directory (`jdbc-drivers/<Artifact>/<version>/…`) `[Observed: help:postgresql "Location for the downloaded JDBC drivers is the DataGrip configuration directory"; local dir exists]`.
- UI: *Download ver. N*, choose a specific version, *Switch to Latest Driver*, custom JARs, `BestDriverValidator` suggests a better driver for the detected server `[Observed: bundle:DataSourceConfigurable.switch.to.latest; classes: BestDriverValidator]`.

Shipped/listed versions of note: Redis driver 1.4, 1.5; MongoDB driver up to 1.20; ClickHouse 0.1.40 … 0.4.x; MySQL Connector/J from 5.1.21 upward `[Observed]`.

### 3.7 Data-source options (per DS) — complete list from option providers

`[Observed: optionProvider registrations + bundle labels + help]`

| Option (UI label) | Provider | Scope | Semantics |
|---|---|---|---|
| Disable incremental introspection | `ForbidIncrementalOptionProvider` | Expert, all | Always full introspection |
| Disable fragment introspection | `ForbidFragmentOptionProvider` | Expert | "Use only when fragment introspection behaves incorrectly" |
| Use IDE proxy settings | `PassProxyOptionProvider` | Expert | Passes IDE HTTP proxy to the driver process |
| Introspect using JDBC metadata | `JdbcIntrospectorOptionProvider` | Expert | Switch to generic introspector |
| (JDBC introspector without escaping) | `JdbcIntrospectorNoEscapingOptionProvider` | Expert | `[Inferred]` do not escape `_`/`%` in metadata name patterns (broken drivers) |
| Isolate class path | `IsolateClassPathOptionProvider` | Expert | Separate class loader for driver |
| Automatic introspection interval N minutes | `IntrospectionScheduleOptionProvider` | Options | Periodic refresh |
| Enable DBMS_OUTPUT | `DbmsOutputOptionProvider` | Oracle, Db2 LUW | Default for new sessions |
| (Desktop actions / browser for SSO) | `DesktopActionsOptionProvider` | auth flows | `[Inferred]` allow the driver process to open a browser (Azure AD interactive, Snowflake external browser, BigQuery user account) |
| DBMS (Forced DBMS) | `ForcedDbmsOptionProvider` | General | "Forcefully set what DBMS you want to use… when the IDE cannot identify the supported DBMS" |
| Automatically assign session_id | `CHouseAutoSessionOption` | ClickHouse | Unique `session_id` per connection so `USE`/`SET` persist across HTTP requests |
| (MariaDB generated expressions via SHOW CREATE TABLE) | `MariaGenExprRetrievalOption` | MariaDB < 10.2.5 | `information_schema.columns.generation_expression` missing in old versions |
| (MySQL routine retrieval mode) | `MysqlRoutineRetrievalOption` | MySQL | `[Inferred]` choose `SHOW CREATE` vs `information_schema.routines` (privileges) |
| (Oracle fetch LONG columns) | `OraFetchLongsOptionProvider` | Oracle | LONG-typed dictionary columns (view text, defaults) cost round-trips |
| Introspect server objects | `OraSkipServerObjectsOptionProvider` | Oracle | Users, tablespaces, data files |
| Do not use xmin in queries to pg_database | `PgNoDatabaseXminOption` | PostgreSQL | Workaround for proxies/forks without `xmin` on `pg_database` |
| Show internal system schemas | `PgSystemSchemasOption` | PostgreSQL | `pg_toast`, `pg_temp_*` |
| Show template databases | `PgTemplateDatabaseOption` | PostgreSQL | `template0/1` |
| Default key filter / COUNT option for SCAN / Key parts delimiter | `RedisDefaultKeyFilterOption`, `RedisScanCountOption`, `RedisHierarchyOption` | Redis | see `05` |
| (Snowflake SHOW statement limit) | `SFlakeShowStatementLimitOption` | Snowflake | `SHOW … LIMIT` for huge accounts |
| (Snowflake stage properties) | `SFlakeStagePropertiesRetrievalOption` | Snowflake | `DESCRIBE STAGE` per stage is slow |
| (SQLite temp dir) | `SqliteTempDirParamProvider$Option` | SQLite | process param for native temp dir |
| Register REGEXP function | `SqliteRegisterRegexpOption` | SQLite | registers a `REGEXP` implementation |
| (LocalDB exec) | `LocalDbExecOption` | LocalDB | path to `SqlLocalDB.exe` |

General/Options tab items (from the help, all DBMSs): Read-only, Transaction control (Auto/Manual + isolation), Switch schema (Automatic/Manual/Disable), Time zone, Single session mode, Run keep-alive query each N s, Auto-disconnect after N s, Single database mode, Startup script (with IDE macros), Auto sync, Load sources for (All / All excl. system / None), Default level (Oracle), Use session template, Warn when editing outdated DDL, Track databases/schemas creation and deletion, Use pre-introspected objects, Automatic introspection interval, Virtual objects and attributes file, Code style, Before connection tasks, Session Templates (override Authentication, Read-only, Startup script, Driver, URL parameters) `[Observed: help:data-sources-and-drivers-dialog; classes: DataSourceSessionTemplatePanel$*Widget]`.

### 3.8 Authentication methods per DBMS

`[Observed: auth-provider classes + help per DBMS + bundle labels]`

| DBMS | Methods (UI order where known) | Provider classes |
|---|---|---|
| PostgreSQL, Greenplum, CockroachDB | pgpass, User & Password, No auth | `PgPassAuthProvider` (order first), `DatabaseCredentialsAuthProvider`, `NoCredentialsAuthProvider` |
| Amazon Redshift | pgpass, User & Password, AWS Profile, No auth; connection types `default` / `IAM cluster/region` / URL only; Okta via driver plugin (help: `connect-to-amazon-redshift-with-okta`) | + `AwsProfileAuthProvider` (`REDSHIFT ∈ AWS_PROFILE_AUTH_SUPPORTED`) |
| MySQL, MariaDB | User & Password, No auth (+ Unix socket connection type, named pipes doc) | default providers; Cloud SQL proxy interceptors |
| SQL Server | User & Password, Kerberos, No auth, Domain credentials (Windows, domain\user), Windows SSO (integrated) | `MsKerberosAuthProvider`, `MsSsoAuthProvider`, `MsJtdsDomainAuthProvider` (order last), validators `MsDomainInUserNameValidator`, `MsAuthArtifactValidator` (native DLL presence) |
| Azure SQL / Synapse | User & Password, Kerberos, Azure AD password, Azure AD interactive (browser to login.microsoftonline.com), Azure AD integrated (DLL on Windows or Kerberos ticket), No auth | `MsAzureADPasswordAuthProvider`, `MsAzureADInteractiveAuthProvider`, `MsAzureADIntegratedAuthProvider` (artifacts `ADAL4J`, `MSAL4J`) |
| Oracle | User & Password, Kerberos, No auth; OCI driver mode needs Instant Client (`OraOciValidator`); wallets for Oracle Cloud (help pages) | `OraKerberosAuthProvider`, `OraTnsNamesSshInterceptor` |
| MongoDB / DocumentDB | User & Password, SCRAM-SHA-1, SCRAM-SHA-256, AWS IAM (access key id, secret, session token), X.509, GSSAPI/Kerberos (principal, service name, service realm, canonicalize host name), PLAIN (LDAP), No auth; `authSource` | `MongoScramSha1AuthProvider`, `MongoScramSha256AuthProvider`, `MongoAWSAuthProvider`, `MongoX509AuthProvider`, `MongoKerberosAuthProvider`, `MongoPlainAuthProvider`; driver property names `authMechanism`, `authSource`, `AWS_SESSION_TOKEN`, `SERVICE_NAME`, `SERVICE_REALM`, `CANONICALIZE_HOST_NAME` `[Observed: oss-mongo DriverPropertyInfoHelper@v1.20]`. (Docs swap the SHA-1/SHA-256 descriptions — a doc bug.) |
| Redis | User & Password (ACL user optional), No auth | default |
| DynamoDB | AWS Profile (profile + region), No auth (default credential chain via URL `jdbc:dynamodb://`) | `AwsProfileAuthProvider` |
| Athena | AWS Profile, User & Password (keys) | same |
| BigQuery | Service account (key file), Google user account (browser), Access & refresh tokens, Application Default Credentials, Identity federation | `BigQuery*AuthProvider` (5) |
| Snowflake | User & Password, external browser / SSO | `SFlakeExternalAuthProvider` |
| Cloud Spanner | Credentials file | `CloudSpannerFileAuthProvider` |
| Cassandra, ClickHouse, SQLite, others | User & Password / No auth (SQLite: none) | default |

**Password storage** modes: Never, Until restart, For session (forgotten on disconnect), Forever (OS keychain/KeePass) `[Observed: help + bundle:data.source.setting.password.storage.*]`; "Set Empty" distinguishes an *empty* password from *no* password `[Observed: bundle:action.DatabasePasswordField.set.empty.text]`. `URL only` connection type stores the URL **in plain text** and logs it — the docs warn to keep secrets in fields `[Observed: help:data-sources-and-drivers-dialog]`.

### 3.9 Connect interceptor pipeline — inventory and order

`[Observed: connectionInterceptor registrations; order attributes]`. IntelliJ ordering semantics: `first`, `last`, `before <id>`, otherwise registration order.

| # | Interceptor | Registered in | Order | Purpose |
|---|---|---|---|---|
| 1 | `PasswordExpirationConnectionInterceptor` | DatabasePlugin.xml | **first** | Detect "password expired" at login; show *Password Expired* dialog (new + confirm); run the DBMS-specific change and retry `[Observed: bundle:password.expired.*]` |
| 1 | `PgPassAuthProvider` | postgres.xml | **first** | Fill password from `.pgpass` when auth = pgpass |
| 1 | `BigQueryServiceAccountAuthProvider` | bigquery.xml | **first** | |
| 2 | `OraTnsNamesSshInterceptor` | oracle.xml | **before ssh** | Resolve a TNS alias to host/port *locally* so the SSH tunnel can forward the real endpoint |
| 3 | `SshConnectionInterceptor` (id `ssh`) | DatabaseConnectivity.xml | — | Open/reuse SSH tunnel(s); rewrite host/port in the URL to `127.0.0.1:<local>` |
| 4 | `LogInterceptor` (+ processParamProvider) | | — | Diagnostics logging of JDBC calls (driver-process params) |
| 5 | `DbExpertOptionsInterceptor` | | — | Apply expert options (proxy pass-through, class-path isolation…) |
| 6 | `DatabaseCredentialsAuthProvider` | | — | User & Password |
| 7 | `SslConnectionInterceptor` (+ processParamProvider, consoleRunContextParametersTuner) | | — | Translate SSL tab into driver properties / JVM keystores; also tune CLI tools (pg_dump) |
| 8 | `PgBaseUrlCheckingInterceptor` | postgres.xml | — | `[Inferred]` validate/normalize PG URL (e.g. multi-host, ssl params) |
| 9 | `*CloudSQLProxyInterceptor` (PG, MySQL, MariaDB, MSSQL; base `BaseCloudSQLProxyInterceptor`) | dialects | — | Configure Google Cloud SQL socket factory |
| 10 | `LocalDbInterceptor` | mssql.ex.xml | — | Start/resolve LocalDB instance pipe |
| 11 | DBMS auth providers (Mongo ×6, MSSQL ×5, Oracle Kerberos, BigQuery ×4, Snowflake, AWS profile, Cloud Spanner) | dialects | — | Only the selected provider acts |
| 12 | `ForcedDbmsConnectionInterceptor`, `DesktopActionsConnectionInterceptor` | DatabasePlugin.xml | — | Forced DBMS; allow browser/desktop actions from driver process |
| 13 | `MsJtdsDomainAuthProvider` | mssql.xml | **last** | Windows domain auth for jTDS |
| 13 | `NoCredentialsAuthProvider` | DatabaseConnectivity.xml | **last** | Terminal fallback "No auth" |

There is also a `DatabaseFastConnectionInterceptor` interface `[Observed: class name]` → `[Inferred]` a cheap-path variant used when reconnecting/creating extra session connections without UI prompts. `processParamProvider` is a separate EP that contributes **driver-process** parameters (JVM flags, env) — SSL, logging, desktop actions, BigQuery ADC, SQLite temp dir use it `[Observed]`.

### 3.10 SSH

`[Observed: help:configuring-ssh-and-ssl, help:settings-tools-ssh-configurations, SshBundle, DatabaseBundle]`
- **Configurations** are global (IDE config) or project-scoped ("Visible only for this project", stored in `.idea`, VCS-shareable); data sources reference them by id (`ssh-properties/ssh-config-id`).
- Fields: host (default localhost), user, port (22), auth type — *Password* (save password), *Key pair (OpenSSH or PuTTY)* with passphrase (save passphrase), *OpenSSH config and authentication agent* (ssh-agent, Pageant); *Parse config file ~/.ssh/config* (for Password/Key pair; always for agent mode); *Send keep-alive messages every N s*; *Strict host key checking* (Yes / Accept New / No / Ask = default); *Hash hosts in known_hosts*; *HTTP/SOCKS proxy* for reaching the SSH server, with optional user/password or "Use global IDE proxy settings".
- Supported key formats: "Only SSH2 keys in OpenSSH format (DSA, RSA, ECDSA or ED25519)" `[Observed: SshBundle credentials.invalid.keypair]`.
- **Jump hosts**: no dedicated UI. The literal `ProxyJump` occurs inside the platform `app.jar` binary (string presence only) `[Observed]`; `[Inferred]` multi-hop works through `~/.ssh/config` `ProxyJump` when config parsing is on. `[Unknown]` ProxyCommand support.
- **Tunnel sharing**: "All created SSH connections are shared between all the data sources that you have in a project" `[Observed: help]`.
- **Local port**: dynamic by default; a fixed local port may be forced. Errors: *"Unable to create multiple SSH tunnels if SSH tunnel configuration contains a forced local port"*, *"Forced local port … is already occupied by SSH tunnel to … (expected …)"* `[Observed: bundle]` ⇒ the tunnel manager creates **one forward per (remote host, port)** found in the URL, keyed and reference-counted.
- **Remote host discovery**: the tunnel target is extracted from the URL (*"Unable to find remote host or port in the URL"*). Name resolution happens on the SSH server side ("the SSH tunnel hostname is resolved on the machine with SSH server").
- **Clusters**: Redis cluster requires *all* node host:port pairs in the URL "to create the necessary SSH tunnels" `[Observed: bundle:error.message.redis.missed.port.forwarding]`; the Redis driver then rewrites node addresses announced by `CLUSTER SLOTS` via `hostAndPortMapping` `[Observed: oss-redis README@v1.5]`.
- **Credential prompts**: *"Please provide SSH credentials"*; failures surface as `SshFailedErrorInfo`; tunnel errors `SshForcedPort*ErrorInfo` `[Observed]`.
- **Kubernetes**: port-forward tab (context, namespace, resource type, resource, host port, container port), plugin-based, not for embedded DBs `[Observed: help:data-sources-and-drivers-dialog]`.

### 3.11 SSL/TLS

- Fields: Use SSL, Copy from (another DS), CA file, Use truststore (IDE — certificates accepted under *Tools | Server Certificates*; JAVA; System), client certificate file, client key file, client key password, Mode: **Require** (encrypt; client cert if given), **Verify CA** (+ chain to root), **Full Verification** (+ hostname) `[Observed: help:configuring-ssh-and-ssl]`.
- PEM recommended; self-signed + newest drivers may fail; workaround is driver downgrade (e.g. MySQL 5.1.40) `[Observed: help]`.
- Registry `database.ssl.use.socket.factory=false` — "Replace default socket factory instead of key stores" `[Observed: DatabaseRegistryKeys.xml]` ⇒ default implementation feeds **key stores** (JVM/driver properties); alternative swaps a socket factory.
- Errors: `NoSslProtocolErrorInfo`, MySQL handshake terminated ("enabledTLSProtocols" — quick fixes *Enable all protocols* / *Remove protocol list*), SQL Server `MsSecureServerErrorInfo`, wrong key password (`UserKeyPasswordErrorInfo`), ClickHouse `CHouseHTTPSErrorInfo` `[Observed]`.
- `[Inferred]` per-DBMS mapping: PG `sslmode=require|verify-ca|verify-full` + `sslrootcert/sslcert/sslkey/sslpassword`; MySQL `sslMode=REQUIRED|VERIFY_CA|VERIFY_IDENTITY`; SQL Server `encrypt=true` + `trustServerCertificate`/`hostNameInCertificate`; Mongo `tls=true` + `tlsAllowInvalidCertificates/Hostnames` (driver properties observed); Redis `ssl=true`, `verifyServerCertificate` + JVM truststore flags (driver README); Cassandra via JVM `-Djavax.net.ssl.*` + `sslenabled=true` URL flag `[Observed: help:how-to-connect-to-cassandra-with-ssl]`.

### 3.12 Proxies, cloud and environment

- *Use IDE proxy settings* passes IDE HTTP proxy to the driver process (useful for HTTP-based drivers: ClickHouse, BigQuery, Snowflake, Athena, Databricks) `[Observed]`.
- **Google Cloud SQL**: dedicated `*.cloud_sql` drivers using the Cloud SQL Java socket factory (`cloudSqlInstance=project:region:instance`), host fixed to localhost/none; artifacts "Cloud SQL Proxy (MySQL 8 | MariaDB | PostgreSQL | SQL Server)" `[Observed]`.
- **AWS**: `AwsProfileAuthProvider` for Redshift/Athena/DynamoDB (`~/.aws/credentials` named profiles); Redshift IAM URLs (`jdbc:redshift:iam://cluster:region/db`); **AWS Advanced JDBC Wrapper** drivers for PG/MySQL/MariaDB (`jdbc:aws-wrapper:…`) `[Observed]` → `[Inferred]` RDS IAM tokens and failover come from the wrapper's plugins configured via properties (e.g. `wrapperPlugins=iam`), not from DataGrip code.
- **Azure**: AD flows via MS driver (`authentication=ActiveDirectoryPassword|Interactive|Integrated`), MSAL4J/ADAL4J artifacts.
- **Before connection** tasks (run configurations/external tools) run sequentially before the first connection — a hook for `cloud-sql-proxy`, `kubectl port-forward`, VPN scripts `[Observed: help]`.

### 3.13 Test Connection and error classification

- Test Connection reports success with **ping time**, keep-alive check result and SSL status: format "Ping: {time}{ (keep-alive query results in error)}\nSSL: {yes/no}" `[Observed: bundle:text.ping.choice.keep.alive.query.results.in.error.choice.ssl]`; failure "Keep-alive query failed" is its own status; results can be copied; the test is cancellable `[Observed: bundle:test.connection.*, classes: DataSourceTestConnectionManager$Status/TestInfo]`.
- **Base error infos** `[Observed: classes: BaseDatabaseErrorHandler$*]`: Credentials, Database, DataSourceProcessExited, DriverClassNotFound, ExecutionDriverClassIsIncompatible, ExecutionDriverClassNotFound, ExecutionDriverClassNotInstantiable, Execution, Headless, IO, MissingDriverClass, MissingNativeLibrary, Native, NoSslProtocol, OutOfMemory, PasswordExpired, PasswordIsAboutExpire, ProtocolFamilyUnavailable (IPv6), ReboundNotPossible, Remote, SQL, SshFailed, SuppressUserStackTrace, Unknown, UnsafeUpdateRolledBack, UnsupportedOperation; `OffsetInfo` locates error positions in SQL.
- **Per-DBMS infos/fixes** `[Observed]`:

| DBMS | Error info | Message / quick fix |
|---|---|---|
| MySQL | `MysqlUnrecognizedTimeZoneErrorInfo` | "Server returns invalid timezone. Need to set 'serverTimezone' property." → **Set time zone** |
| MySQL | `MysqlHandshakeTerminatedErrorInfo`, `MysqlNoSslProtocolErrorInfo` | TLS protocol list → *Enable all protocols in JDBC driver* / *Remove protocol list* |
| PostgreSQL family | `PgBouncerNoPreparedStatements` | PgBouncer (transaction pooling) breaks server-side prepared statements → `[Inferred]` fix sets `prepareThreshold=0` |
| Oracle | `OraMissingOCIErrorInfo`, `OraNlsProblemErrorInfo` ("client process and server locale differ"), `OraNativeErrorInfo`, `OraWrappedDatabaseErrorInfo`; remote `OracleTimezoneRegionNotFoundException` | install Instant Client; set `NLS_LANG`; timezone flags |
| SQL Server | `MsJbDriverSQLErrorInfo`, `MsJbDriverUnexpectedErrorInfo`, `MsSecureServerErrorInfo` | TLS/encrypt hints |
| SQLite | `SqliteWSLFileLocked` | DB file on WSL path locked |
| Redis | `RedisModeMatchingErrorInfo` (fix *Use "jdbc:redis:cluster://"*), `RedisModeRetrievingErrorInfo` (fix *Disable connection mode verification* → `verifyConnectionMode=false`), `RedisPortForwardingErrorInfo`, `RedisOutdatedDriverErrorInfo`, `RedisColumnHintErrorInfo`, `RedisUnknownCommandErrorInfo` | see `05` |
| ClickHouse | `CHouseHTTPSErrorInfo`, `CHouseNativeErrorInfo` | HTTPS port/protocol mismatch |
| Couchbase | `CouchCauseHasNoDocument`, `CouchOriginalHasNoDocument` | |
| BigQuery | `BigQueryTableNotExistErrorInfo` | |
| Cassandra, Derby, Hive, HSQLDB, Redshift, Vertica | `*NativeErrorInfo` | vendor code extraction |

- **Remote exception taxonomy** recognized in the driver process (`*ExceptionRecognizer` per DBMS): `DBLoginFailedException`, `DBAccessDeniedException`, `DBSchemaAccessDeniedException`, `DBColumnAccessDeniedException`, `DBConnectionException`, `DBIsNotConnected`, `DBSessionIsClosedException`, `DBServiceIsNotActiveException`, `DBTransactionIsAlreadyStartedException`, `DuplicateKeyException`, `NoRowsException`, `NoTableOrViewException`, `OracleObjectNotFoundException`, `DbmsUnsupportedFeatureException`, … `[Observed]`.
- After a credentials error the IDE re-asks credentials (*"The specified database user/password combination is rejected"*, progress "Re-ask credentials") `[Observed]`.

### 3.14 Configuration validators ("Problems")

`[Observed: configValidator registrations]` `LocalDataSourceValidator`, `SshDataSourceValidator`, `BestDriverValidator`, `AuthProviderValidator`, `DatabaseDriverValidator`, `DdlMappingSourceValidator`, `SqlDataSourceValidator`, `MsDomainInUserNameValidator` (user typed `DOMAIN\user` with non-domain auth), `MsAuthArtifactValidator` (native auth DLL missing), `MysqlSocketParamEditor$Validator`, `OraOciValidator`, `SqliteConfigValidator`; plus `urlEditorInspector` `SpacesInspector` (whitespace in URL). Problems appear as a counter with fix buttons `[Observed: help "Problems"]`. Messages: "No database URL provided", "No driver files provided", "Driver files are not downloaded", "Unknown provided driver" `[Observed: bundle:DataSourceConfigurable.*]`.

### 3.15 Data-source detection and import

`DataSourceDetector` scans project files (config files) and notifies "Database connection parameters found → Connect to Database / Do not suggest again" `[Observed: bundle:data.sources.detected*; classes: autoconfig/*]`. Data sources can be copied/pasted as XML (no password unless in a URL-only URL) and exported/imported via IDE settings (global DS only) `[Observed: help:managing-data-sources]`.

---

## 4. Algorithm specifications

### ALG-URL-01: Template compilation

- **Problem:** Turn a template string into an AST usable for both rendering and parsing.
- **Inputs / outputs:** `&str` → `Template { nodes: Vec<Node> }` or `TemplateError{pos,msg}`.
- **Algorithm (recursive descent):**
  1. `parse_seq(stop_chars)` reads nodes until an unescaped stop char (`]`, `>`, `|`, `,` depending on context) or end.
  2. `\x` → `Literal(x)`.
  3. `{` → read until matching unescaped `}`; split on unescaped `:` into up to 5 fields `[name, type, default, label, escape]`; if `type` contains `/`, split first `/` into `type` and `regex`; strip leading `#`/`##` from `type` into `visibility` and leading `#` from `name` into `fixed=true`; empty name → `anonymous=true`. For `variants`, field 4 is `Label=value,…` (split on unescaped `,`, then `=`).
  4. `[` → `Optional(alts)` where `alts = parse_seq(']','|')` repeated on `|`.
  5. `<` → read separator up to first unescaped `,` → `List{sep, alts}` with alternatives = comma-separated `parse_seq('>' ',')`; an alternative starting with `!!` is `mandatory`.
  6. `!(` → read literal until `)` → `NotAhead(text)`.
  7. `?` directly after a node → wrap previous node in `MaybeEmpty`.
  8. Any other char → `Literal`.
- **Data structures:** `enum Node { Lit(String), Ph(Placeholder), Opt(Vec<Vec<Node>>), List{sep:String, alts:Vec<Vec<Node>>}, NotAhead(String), MaybeEmpty(Box<Node>) }`; `Placeholder{name:Option<String>, ty:PhType, regex:Option<Regex>, default:Option<String>, label:Option<String>, escape:Escape, visibility:Vis, fixed:bool}`.
- **Complexity:** O(n).
- **Edge cases:** escaped colons in defaults (`\:\:1`); `{:identifier:::%}` (anonymous with escape); nested optionals `[[{user}:]{password}@]`; list separator that is itself escaped (`\,`) or multi-char (`\)\(`); `user[Name]` optional literal; empty template (`<url-template/>` = reset inherited list); unbalanced brackets → error with position; variants value containing escaped colon (`iam\:`).
- **Test cases:** `"jdbc:x://{host::localhost}[:{port::5432}]"` → `[Lit, Ph(host,default=localhost), Opt([Lit(":"), Ph(port,default=5432)])]`; `"{host:ipv6:\:\:1}"` → default `::1`; `"<\,,{h}>"` → `List{sep:",",…}`; `"{database:::keyspace}"` → label `keyspace`; `"[a|b]"` → `Opt([[a],[b]])`; `"{x"` → error at 0.
- **Prior art and evidence:** `[Observed: 107 templates in databaseDrivers/*.xml]`; grammar `[Inferred]`.
- **Used by:** Hacivat (connection dialog), Perde (descriptor validation).

### ALG-URL-02: Render form → URL

- **Problem:** Produce a URL from field values, extra properties and a chosen template.
- **Inputs / outputs:** `Template`, `fields: Map<name,String>`, `extra: Vec<(k,v)>` (ordered) → `String`.
- **Algorithm:**
  1. Walk nodes. `Lit` → append. `Ph` named → value or default; if value empty and node is not inside an optional → append default (or nothing if no default and `MaybeEmpty`); apply escape (`%` → percent-encode reserved chars of the URL component; `[` → driver-specific escaping) ; anonymous Ph → its default.
  2. `Opt(alts)` → render first alternative in which **at least one named placeholder has a non-empty user value**; else render nothing. (Literal-only alternatives like `[http://|https://]` → render nothing unless a field `scheme` selects one.)
  3. `List{sep, alts}` → build items: for each named placeholder alternative (e.g. `user={user}`) whose value is non-empty emit one item; then for every `extra` key not already emitted, emit using the passthrough alternative `{:identifier}={:param}`; `mandatory` items always; join with `sep`. If no items, the list renders empty (enclosing optional disappears).
  4. Host lists (`<\,,{host}[:{port}]>`) take `fields.hosts: Vec<(host, port)>`.
  5. `NotAhead` renders nothing.
- **Edge cases:** passwords must **not** be rendered into the URL when they are passed as properties (DataGrip keeps `password={password:param}` but stores secrets separately; Hacivat: render secrets only into the in-memory URL, never into persisted/displayed URL); IPv6 literal requires the bracket template; default database (`postgres`) vs explicit empty; percent-encoding of `@`, `/`, `?`, `#`, `%`, `:` in user/password/database; duplicate extra keys; key order stability (keep insertion order to make diffs stable).
- **Test cases:** PG `{host:"db",port:"",database:"app"}` → `jdbc:postgresql://db/app`; with `extra=[("sslmode","require")]` → `…/app?sslmode=require`; user `a@b` with `%` escape → `a%40b`; Redis cluster hosts `[(h1,7000),(h2,7001)]` → `jdbc:redis:cluster://h1:7000,h2:7001`; Oracle Service Name `{host:"o",service:"ORCLPDB"}` with driver=thin → `jdbc:oracle:thin:@//o/ORCLPDB`.
- **Prior art:** DataGrip General tab ↔ URL field synchronisation `[Observed: help]`.
- **Used by:** Hacivat; Karagöz (when it publishes discovered endpoints as descriptors, it should emit *fields*, not URLs).

### ALG-URL-03: Parse URL → form (multi-template matching)

- **Problem:** A user pastes a URL; find the template + field values; unknown params → extra properties.
- **Inputs / outputs:** `url: &str`, `templates: &[Template]` (driver's, in order, plus all other drivers' for auto-switching) → `Option<(template_idx, fields, extra)>`.
- **Algorithm:** backtracking matcher (PEG-style with backtracking on optionals/alternatives):
  1. `match(nodes, pos)` returns all/first successful `(pos', bindings)`.
  2. `Lit` must match case-sensitively (prefix `jdbc:` optional only where template says `[jdbc:]`).
  3. `Ph`: greedy-then-backtrack over the maximal span allowed by `regex` (default regex per type: `host` = `[^:/?;,\[\]]+`, `port` = `[0-9]+`, `param` = `[^&;]*`, `identifier` = `[^&;=,:/]+`, `path` = `.+?`). Unescape according to `escape` (`%` → percent-decode).
  4. `Opt` → try each alternative, then the empty match.
  5. `List` → repeatedly match an alternative, then `sep`, until no alternative matches; named alternatives bind to fields, passthrough binds to `extra`; a key that matches a named alternative goes to the field even if written in another case (`userName` via `user[Name]`).
  6. `NotAhead(t)` → fail if input at pos starts with `t`.
  7. Success requires consuming the whole input. Choose the **first template** in declared order that succeeds; tie-break by fewest `extra` entries.
  8. If none of the selected driver's templates match, try other drivers' templates; if one matches, propose switching driver (DataGrip changes driver on paste of e.g. a `jdbc:mysql:` URL into a PG data source `[Inferred]`); else keep as `URL only`.
- **Complexity:** worst-case exponential with nested optionals; cap backtracking steps (e.g. 10⁵) and memoize `(node_id,pos)`.
- **Edge cases (exhaustive):** IPv6 `[::1]` vs optional group brackets; `jdbc:postgresql:dbname` (no `//`); SQL Server `server\instance` vs `;instanceName=`; `userName=`/`user=` duplicates (last wins + warning); empty values `?user=`; trailing separator `;`; repeated keys; percent-encoded separators inside values (`%26`); Mongo `mongodb+srv` (no port allowed); `jdbc:` prefix omitted (Mongo template allows); Oracle Long descriptor with arbitrary whitespace and nested parentheses (needs a balanced-paren scanner for `ora_parens`); Unicode host names (IDN) — keep as typed; password containing `@` in `user:pass@host` forms (split on **last** `@` before host); `?` inside a database name (excluded by regex `[^?]+`).
- **Test cases:**
  - `jdbc:postgresql://h:6543/d?sslmode=verify-full&user=u` → template 1, `{host:h, port:6543, database:d, user:u}`, `extra=[sslmode=verify-full]`.
  - `jdbc:postgresql://[::1]/d` → template 2 (IPv6).
  - `jdbc:postgresql:d` → template 3.
  - `jdbc:sqlserver://srv\SQLEXPRESS;databaseName=x;encrypt=true` → template 1, `instance=SQLEXPRESS`, `database=x`, extra `encrypt=true`.
  - `jdbc:redis:cluster://a:1,b:2/0` → cluster template, hosts list size 2.
  - `jdbc:sqlite::memory:` → second template (first excluded by `!(:memory:)`).
  - `jdbc:h2:mem:test` → In-memory (Embedded excluded by `!(mem:)`).
  - `mongodb+srv://u:p@cluster0.x.net/db?retryWrites=true` → SRV template, extra `retryWrites`.
  - `garbage` → `None` → URL only.
- **Prior art and evidence:** `[Observed: templates, help "paste the JDBC URL in the URL field"]`.
- **Used by:** Hacivat.

### ALG-CONN-01: Connect interceptor pipeline

- **Problem:** Turn a data-source descriptor into a live connection, applying tunnels, TLS, auth and environment in a deterministic order, with cleanup on failure.
- **Inputs / outputs:** `DataSource`, `Requestor{department, client}` → `Result<LiveConnection, ConnectError>`.
- **Algorithm:**
  1. Build `ConnectRequest { url_fields, props, process_params, secrets: lazy, dbms_hint }`.
  2. Sort steps by `(phase, order)` where phases = `[PreAuth(first), NameResolution(before ssh), Transport(ssh, k8s, cloud proxy), Security(ssl), Auth(selected provider), Environment(expert, forced dbms, logging), Fallback(last)]`.
  3. For each step: `if step.applies(&req)` → `step.apply(&mut req) -> Result<Option<Cleanup>>`; push cleanup on a stack.
  4. Render URL (ALG-URL-02) from possibly rewritten host/port.
  5. Driver connect with timeout; on error → classify (ALG-CONN-04). If `PasswordExpired` and interactive → prompt new password, run change, retry once. If `Credentials` and interactive and storage ≠ Forever-valid → re-ask, retry. If `TunnelBroken` → drop tunnel from cache, retry once.
  6. On final failure run cleanups LIFO (close tunnels if refcount hits 0).
  7. Post-connect: detect DBMS (product/version), persist into local config, run session environment (see `03` ALG-SESS-01).
- **Data structures:** `trait ConnectStep { fn phase(&self)->Phase; fn order(&self)->i32; fn applies(&self,&ConnectRequest)->bool; async fn apply(&self,&mut ConnectRequest)->Result<Option<Cleanup>>; }`.
- **Edge cases:** steps requiring UI (password, SSO browser) in non-interactive contexts (introspection in background, Karagöz-side) must fail fast with a typed error; SSO steps must open the browser from the *UI process*; cancel during connect (DataGrip supports "Connection canceled" `[Observed]`); two concurrent connects for the same DS must share the tunnel and the credential prompt (single-flight); secret zeroization after use; Oracle TNS before SSH; pgpass before credentials; forced DBMS overriding detection.
- **Test cases:** PG + SSH + pgpass → steps applied in order pgpass→ssh→ssl→auth; failure in SSL step closes the tunnel opened by SSH; expired password path retries exactly once; cancel during SSH handshake returns `Cancelled` and leaves no tunnel.
- **Prior art and evidence:** `[Observed: connectionInterceptor registrations, order attributes; help:run-a-query "Cancel connections"]`.
- **Used by:** Hacivat; Karagöz (server side: credential injection step replaces user-typed secrets).

### ALG-CONN-02: Shared SSH tunnel manager (multi-host)

- **Problem:** Reuse SSH sessions and port forwards across data sources/sessions; support clusters (one forward per node).
- **Inputs / outputs:** `(ssh_config_id, Vec<(remote_host, remote_port)>, forced_local_port?)` → `Vec<(local_addr)>` + handles.
- **Algorithm:**
  1. Key SSH *session* by `ssh_config_id` (+ resolved config hash); key *forward* by `(session_key, remote_host, remote_port)`.
  2. If `forced_local_port` and `len(targets) > 1` → error "multiple tunnels with forced port" `[Observed message]`.
  3. For each target: if forward exists and alive → `refcount += 1`; else bind `127.0.0.1:0` (or forced port; if occupied by another forward with different target → error `[Observed message]`), start forwarding.
  4. Return the mapping `remote → local` so the URL renderer and **cluster address mappers** (Redis `hostAndPortMapping`, Mongo replica-set hosts) can rewrite announced addresses.
  5. Keep-alive per SSH session; on session drop mark all its forwards broken; next acquire reconnects.
  6. Release decrements; idle forwards closed after a grace period (e.g. 30 s) to absorb reconnect storms.
- **Edge cases:** cluster topology changes (new node not tunnelled → explicit error suggesting adding the node, or dynamic SOCKS); host-key change (policy: Ask/Yes/No/AcceptNew); agent unavailable; jump hosts (ProxyJump chain = nested sessions keyed by chain); IPv6 remote targets; name resolution must be **remote** (send hostname in direct-tcpip request, never resolve locally).
- **Test cases:** two data sources same SSH config + same DB → one forward, refcount 2; Redis cluster 3 nodes → 3 forwards + mapping; forced port with 2 targets → error; SSH drop → reacquire creates new forward with new local port and sessions reconnect.
- **Prior art:** `[Observed: help "shared between all data sources", bundle tunnel errors, Redis cluster requirement]`.
- **Used by:** Hacivat. (Karagöz/Perde path replaces SSH for managed DBs.)

### ALG-CONN-03: pgpass resolution

- **Problem:** Find a password in `~/.pgpass` / `%APPDATA%\postgresql\pgpass.conf` / `$PGPASSFILE`.
- **Inputs / outputs:** `(host, port, database, user)` after SSH rewriting? → `Option<Secret>`.
- **Algorithm:** read file; ignore if permissions are group/world-readable on Unix (libpq behaviour); for each non-comment line split on unescaped `:` into 5 fields with `\:` and `\\` unescaping; first line where each of host/port/db/user equals the value or is `*` wins. Host matching uses the **original** host (before SSH rewrite) — `[Inferred]` this is why pgpass runs *first*. `localhost` also matches Unix-socket connections.
- **Edge cases:** CRLF; trailing whitespace; missing fields (skip line); empty password field; `database` = `replication`; default port 5432 when absent; Windows path; file absent.
- **Test cases:** line `*:*:*:u:pw` matches any host for user `u`; `db\:1:5432:*:*:x` matches host `db:1`; file mode 0644 → ignored with warning.
- **Prior art:** PostgreSQL libpq docs; `[Observed: PgPassAuthProvider order=first]`.
- **Used by:** Hacivat.

### ALG-CONN-04: Error classification and quick-fix selection

- **Problem:** Map heterogeneous driver errors to a stable taxonomy, localized message, and actionable fixes.
- **Inputs / outputs:** `(dbms, phase{connect|execute|introspect}, error{sqlstate?, vendor_code?, message, cause_chain, io_kind?})` → `Classified{class, offset?, fixes: Vec<FixId>, retryable: bool}`.
- **Algorithm:** ordered rule table per DBMS, then base rules:
  1. DBMS rules by `(sqlstate, vendor_code)` (PG `28P01` invalid password, `28000`; PG `28P01`+"expired" / `PASSWORD EXPIRED`; MSSQL 18456 login failed, 18487/18488 expired/must change; Oracle `ORA-01017`, `ORA-28001` expired, `ORA-28002` grace (about to expire), `ORA-01882` tz region; MySQL 1045 access denied, 1862 password expired; Mongo `AuthenticationFailed` 18).
  2. Message regex rules (MySQL "The server time zone value … is unrecognized" → `UnrecognizedTimeZone` + fix `SetServerTimezone`; TLS "handshake terminated"/"No appropriate protocol" → TLS fixes; Redis "mode mismatch" → `UseClusterPrefix`/`DisableModeVerification`; PgBouncer "prepared statement … does not exist" → `DisablePreparedStatements`).
  3. Base rules: unknown host / connection refused / timeout (IO, retryable); `ClassNotFound` (driver missing → `DownloadDriver`); native library missing; OOM in driver host (`IncreaseMemory`); process exited (`RestartDriverHost`); SSH failure; unsupported operation.
  4. Extract SQL offset (PG `position`, MySQL "near '…' at line N", Oracle line/col) for editor highlighting.
- **Edge cases:** wrapped causes (search full chain); localized server messages (match codes before text!); errors after partial results; errors inside SSH vs DB; same code different meanings across forks (Redshift vs PG).
- **Test cases:** MySQL tz error → fix `SetServerTimezone` sets `serverTimezone=<local tz>` property; Oracle `ORA-28002` → warning class `PasswordAboutToExpire` (connect succeeds); Redis `ERR This instance has cluster support disabled` with cluster URL → `UseStandalonePrefix`.
- **Prior art:** `[Observed: BaseDatabaseErrorHandler$*, per-DBMS ErrorInfo classes, bundle error.message.*]`; codes from vendor docs.
- **Used by:** Hacivat; Karagöz (audit classification).

### ALG-CONN-05: Connection-bounds rewrite (connect to another database on the same server)

- **Problem:** PG/MSSQL/Redshift need a separate physical connection per database to introspect or query it; MySQL just switches schema.
- **Inputs / outputs:** `DataSource`, `target_db` → `ConnectRequest` (or a `USE`/`SET` action).
- **Algorithm:** read driver `connection-bounds`: if `bound-to=schema` (MySQL) → no new connection, issue schema switch (allowed even in read-only when `in-read-only=true`); else clone request, replace each `rewrite-parameter` (URL path placeholder `database` when `in-url=true`, and properties like `PGDBNAME`, `databaseName`) with `target_db`; if `target_db` empty use `default-value` (`postgres`, `master`, `defaultdb`).
- **Edge cases:** Single database mode forbids other DBs (*"The only accessible database in the single database mode is the current database"* `[Observed: bundle:base.introspector.switching.database.singleDatabaseMode.description]`); PgBouncer configured per-DB; Azure SQL forbids `USE` → must reconnect; template DBs with `datallowconn=false` → skip.
- **Test cases:** PG DS on `app` introspecting `analytics` → URL path `analytics`; MySQL → `USE analytics`; Azure single-db mode → error.
- **Prior art:** `[Observed: connection-bounds in postgres/azure/mysql/cockroach/redshift drivers]`.
- **Used by:** Hacivat; Karagöz (per-database authorization must consider that one DS can open many DB connections).

### ALG-CONN-06: Driver/version selection by server constraints

- **Problem:** Pick a compatible driver artifact version for a detected server version.
- **Algorithm:** after first successful connect (or from a version probe), evaluate `constraint`s of candidate driver records in the same `group`/family: choose the record whose `[from,to)` contains the server version; within an artifact, filter versions by `db-from/db-to`, `ide-to`; prefer `use=true` + `rolling` (latest compatible); otherwise pinned. Suggest switching if current driver violates constraints (BestDriverValidator behaviour).
- **Edge cases:** version unknown before first connect (use default/latest, then re-evaluate); forks reporting fake versions (TiDB reports MySQL 5.7/8.0); downgrade loops; offline (no download) → keep current with warning.
- **Test cases:** MySQL 5.1 server → `mysql` (5.1 driver); Mongo 3.6 → `mongo` (3.x driver); ClickHouse 20.3 server → legacy driver ≤ 0.3.1.
- **Prior art:** `[Observed: constraints in drivers and artifacts XML]`.
- **Used by:** Hacivat (Rust: feature-gated backends + a server capability matrix rather than jar downloads).

---

## 5. Implications for Hacivat / Karagöz / Perde

1. **Ship a declarative driver catalog** (TOML/JSON, versioned schema) with the same concepts: id, family, templates (own syntax, but keep *bidirectional* semantics and connection types), default properties *with a rationale field* (so the "why" in §3.5 is kept with the data), keep-alive query, constraints, connection-bounds. Validate it in CI with the test vectors above.
2. **Rust backends** replace JDBC drivers; "default properties" become *backend options* (e.g. `tokio-postgres` `application_name`, MySQL `tinyint1_as_bool=false`-equivalent handling in our value decoder, MSSQL `encrypt` default chosen per target (Azure vs on-prem)).
3. **Auth providers as `ConnectStep`s** (ALG-CONN-01) including pgpass, AWS profile + RDS IAM token generation (SigV4, 15-min tokens refreshed on reconnect), Azure AD (device code + interactive), GCP (ADC), Kerberos (feature-gated), Mongo mechanisms (native in the Rust driver), SSO browser steps executed in the UI process.
4. **SSH tunnel manager** (ALG-CONN-02) with remote resolution, ProxyJump chains (explicit UI, unlike DataGrip), shared forwards and cluster address mapping.
5. **Error taxonomy** shared with Karagöz audit (same class ids). Prefer SQLSTATE/vendor codes over message text (localized servers!).
6. **Perde-managed data sources**: Karagöz exposes endpoints; Hacivat's pipeline gets a `PerdeTransport` step (P2P QUIC / relay) in the Transport phase, and a `KaragozAuth` step that requests a short-lived grant instead of user credentials. The rest of the pipeline (TLS to DB, session env) stays identical.
7. Never persist secrets in the shareable config; "URL only" must still extract secrets into the keyring (improve on DataGrip's plain-text warning).

## 6. i18n considerations

- Connection types, auth method names, option labels, error classes and quick-fix titles are user-facing → message keys (EN+TR). Keep **template names as stable ids** (`default`, `unix_socket`, `cluster`) and translate labels separately (DataGrip uses the English template name as UI label — not localizable).
- Error *matching* must never depend on localized server messages; Turkish-locale servers (e.g. SQL Server, Oracle with `NLS_LANGUAGE=TURKISH`) return Turkish text. Use codes; fall back to text only for drivers without codes.
- Turkish case folding: URL keys like `user`/`USER` (Informix uses `USER=`) must be compared ASCII-case-insensitively; `"INFORMIX".to_lowercase()` under tr locale is a classic bug source (`I`→`ı`).
- Password prompts and expiry dialogs: ensure RTL-safe/IME-safe password fields; error messages with parameters (`{0}`) need ICU/Fluent placeables.

## 7. Risks, pitfalls, open questions

- `[Unknown]` Exact meaning of escape mode `[` and visibility markers `#`/`##` in templates; `!!` list items.
- `[Unknown]` Jump-host support beyond `~/.ssh/config`.
- `[Unknown]` Default keep-alive interval and auto-disconnect values (not in shipped resources).
- `[Inferred]` RDS IAM for PG/MySQL relies on AWS wrapper plugins; DataGrip has no own RDS token generator class (no `Rds`/`Iam` auth-provider class names observed).
- Risk: templates allow ambiguous parses (e.g. PG template 1 vs 3) — Hacivat must keep deterministic order and tests.
- Risk: storing detected server info (`database-info`) locally means stale DBMS detection after upgrades → re-detect on each connect, like DataGrip.
