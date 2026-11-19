CREATE TABLE hidden_gen (
    a int,
    g int GENERATED ALWAYS AS ((`a` + 1)) VIRTUAL INVISIBLE
);
