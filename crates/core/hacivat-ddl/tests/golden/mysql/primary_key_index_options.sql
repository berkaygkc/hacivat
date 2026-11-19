CREATE TABLE daily (
    region char(2) NOT NULL,
    day date NOT NULL
);

ALTER TABLE daily ADD PRIMARY KEY (region, day DESC) COMMENT 'Newest day first';
