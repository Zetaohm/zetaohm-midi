#!/bin/bash

# view-mermaid-simple.sh - View Markdown and Mermaid files in Kitty terminal
# without Chrome dependencies

set -e

# Create temp directory
TEMP_DIR=$(mktemp -d)

# Generate renderer script
cat >"$TEMP_DIR/render.js" <<'EOL'
const fs = require('fs');
const path = require('path');
const mermaid = require('mermaid');

// Initialize mermaid
mermaid.initialize({ startOnLoad: false });

async function renderDiagram(inputFile, outputFile) {
  const content = fs.readFileSync(inputFile, 'utf-8');
  const { svg } = await mermaid.render('mermaid', content);
  fs.writeFileSync(outputFile, svg);
  console.log(```Rendered to ${outputFile}```);
}

const inputFile = process.argv[2];
const outputFile = process.argv[3];
renderDiagram(inputFile, outputFile).catch(console.error);
EOL

# Install necessary dependencies once
if [ ! -d "$HOME/.mermaid-renderer" ]; then
	mkdir -p "$HOME/.mermaid-renderer"
	cd "$HOME/.mermaid-renderer"
	npm init -y
	npm install mermaid
	echo "Installed mermaid renderer dependencies"
fi

# Process mermaid file and display with kitty
process_mermaid() {
	local file="$1"
	local svg_output="$TEMP_DIR/output.svg"

	echo "Rendering Mermaid diagram..."
	node "$TEMP_DIR/render.js" "$file" "$svg_output"
	kitty icat "$svg_output"
}

# Process a markdown file and extract mermaid blocks
process_markdown() {
	local file="$1"

	# Display markdown content
	echo -e "\n=== MARKDOWN CONTENT ===\n"
	if command -v bat &>/dev/null; then
		bat --style=plain --color=always "$file"
	else
		cat "$file"
	fi

	echo -e "\n=== MERMAID DIAGRAMS ===\n"

	# Find and process mermaid blocks - using single quotes in grep
	local counter=0
	grep -n '^```mermaid' "$file" | while read -r line; do
		start_line=$(echo "$line" | cut -d: -f1)
		end_line=$(tail -n +$((start_line + 1)) "$file" | grep -n '^```' | head -n 1 | cut -d: -f1)

		if [ -n "$end_line" ]; then
			end_line=$((start_line + end_line))
			counter=$((counter + 1))

			temp_mmd="$TEMP_DIR/diagram_${counter}.mmd"
			sed -n "$((start_line + 1)),$((end_line - 1))p" "$file" >"$temp_mmd"

			if [ -s "$temp_mmd" ]; then
				echo -e "\nDiagram #${counter} (from line $start_line):"
				local svg_output="$TEMP_DIR/diagram_${counter}.svg"
				node "$TEMP_DIR/render.js" "$temp_mmd" "$svg_output"
				kitty icat "$svg_output"
				echo -e "\n"
			fi
		fi
	done

	if [ "$counter" -eq 0 ]; then
		echo "No Mermaid diagrams found in this Markdown file."
	fi
}

# Main function
main() {
	if [ $# -eq 0 ]; then
		echo "Usage: $0 <file.mmd or file.md>"
		exit 1
	fi

	local file="$1"

	if [ ! -f "$file" ]; then
		echo "Error: File '$file' not found."
		exit 1
	fi

	if [[ "$file" == *.mmd ]]; then
		process_mermaid "$file"
	elif [[ "$file" == *.md ]]; then
		process_markdown "$file"
	else
		echo "Unsupported file type. Please use .mmd or .md files."
		exit 1
	fi

	# Clean up
	rm -rf "$TEMP_DIR"
}

main "$@"
