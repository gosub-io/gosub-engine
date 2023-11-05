FILE_NAME = 'AUTHORS'

with open(FILE_NAME, 'r') as f:
    lines = [line.strip() for line in f.readlines()]


header: list[str] = []
authors: list[str] = []

for line in lines:
    if line == '' or line.startswith('#'):
        header.append(line)
    else:
        authors.append(line)


sorted_lines = header + sorted(authors)

with open(FILE_NAME, 'w') as f: 
    f.write('\n'.join(sorted_lines))
