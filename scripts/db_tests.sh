#!/bin/bash

# Define the path to the file and schema
FILE_PATH="db/dev.db" # Replace with the path to the file you want to delete
SCHEMA_PATH="db/schema.prisma"

# Step 1: Delete the file if it exists
if [ -f "$FILE_PATH" ]; then
    echo "Deleting file: $FILE_PATH"
    rm "$FILE_PATH"
else
    echo "No file found at: $FILE_PATH"
fi

# Step 2: Run Prisma migrate
echo "Running Prisma migrate..."
cargo run -p prisma migrate dev --schema="$SCHEMA_PATH"

# Step 3: Run Prisma generate
echo "Running Prisma generate..."
cargo run -p prisma generate --schema="$SCHEMA_PATH"

# Step 4: Run tests
echo "Running integration tests..."
cargo test -p db --lib db_tests -- --nocapture
