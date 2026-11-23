CREATE TABLE IF NOT EXISTS company_projection (
    company_id VARCHAR(128) CHARACTER SET ascii COLLATE ascii_bin PRIMARY KEY,
    generation BIGINT NOT NULL,
    expires_at BIGINT NOT NULL,
    snapshot MEDIUMTEXT NOT NULL
) ENGINE=InnoDB;

CREATE TABLE IF NOT EXISTS company_account (
    company_id VARCHAR(128) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    provider VARCHAR(32) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    account_ref VARCHAR(128) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    status VARCHAR(8),
    revision BIGINT,
    PRIMARY KEY (company_id, provider, account_ref),
    CONSTRAINT valid_status CHECK (
        (status IS NULL AND revision IS NULL) OR
        (status IS NOT NULL AND revision IS NOT NULL AND status IN ('active', 'paused') AND revision > 0)
    )
) ENGINE=InnoDB;

CREATE TABLE IF NOT EXISTS status_transition (
    company_id VARCHAR(128) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    provider VARCHAR(32) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    account_ref VARCHAR(128) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    revision BIGINT NOT NULL,
    actor VARCHAR(64) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    previous_status VARCHAR(8),
    status VARCHAR(8) NOT NULL,
    committed_at BIGINT NOT NULL,
    request_id VARCHAR(36) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    PRIMARY KEY (company_id, provider, account_ref, revision)
) ENGINE=InnoDB;

CREATE TABLE IF NOT EXISTS accepted_status_write (
    company_id VARCHAR(128) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    provider VARCHAR(32) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    account_ref VARCHAR(128) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    actor VARCHAR(64) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    request_id VARCHAR(36) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    desired_status VARCHAR(8) NOT NULL,
    expected_revision BIGINT,
    accepted_revision BIGINT NOT NULL,
    PRIMARY KEY (company_id, provider, account_ref, actor, request_id)
) ENGINE=InnoDB;
