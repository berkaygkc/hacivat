CREATE TABLE ranked (
    id int NOT NULL,
    score int NOT NULL
);

CREATE UNIQUE INDEX uq_score ON ranked (score DESC) USING HASH COMMENT 'Leaderboard order' INVISIBLE;
