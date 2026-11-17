import { Badge } from "@astryxdesign/core/Badge";
import { Table } from "@astryxdesign/core/Table";
import { TableBody } from "@astryxdesign/core/Table";
import { TableCell } from "@astryxdesign/core/Table";
import { TableHeader } from "@astryxdesign/core/Table";
import { TableHeaderCell } from "@astryxdesign/core/Table";
import { TableRow } from "@astryxdesign/core/Table";
import { Text } from "@astryxdesign/core/Text";
import { flexRender } from "@tanstack/react-table";
import type { LegacyReactTable } from "@tanstack/react-table/legacy";

import type { RequestLogDto } from "@/api/types";

interface LogsTableProps {
  table: LegacyReactTable<RequestLogDto>;
  onSelect: (log: RequestLogDto) => void;
}

export function LogsTable({ table, onSelect }: LogsTableProps) {
  const headerGroups = table.getHeaderGroups();
  const rows = table.getRowModel().rows;

  return (
    <Table density="spacious" dividers="rows" textOverflow="truncate" hasHover>
      <TableHeader>
        {headerGroups.map((headerGroup) => (
          <TableRow key={headerGroup.id} isHeaderRow>
            {headerGroup.headers.map((header) => (
              <TableHeaderCell key={header.id}>
                {header.isPlaceholder
                  ? null
                  : flexRender(
                      header.column.columnDef.header,
                      header.getContext(),
                    )}
              </TableHeaderCell>
            ))}
          </TableRow>
        ))}
      </TableHeader>
      <TableBody>
        {rows.length === 0 ? (
          <TableRow>
            <TableCell colSpan={headerGroups[0]?.headers.length ?? 1}>
              <Text type="supporting" size="sm">
                No requests match the current filters.
              </Text>
            </TableCell>
          </TableRow>
        ) : (
          rows.map((row) => (
            <TableRow key={row.id} onClick={() => onSelect(row.original)}>
              {row.getVisibleCells().map((cell) => (
                <TableCell key={cell.id}>
                  {flexRender(cell.column.columnDef.cell, cell.getContext())}
                </TableCell>
              ))}
            </TableRow>
          ))
        )}
      </TableBody>
    </Table>
  );
}

export function StatusBadge({ statusCode }: { statusCode: number }) {
  const variant =
    statusCode < 400 ? "success" : statusCode < 500 ? "warning" : "error";
  return <Badge variant={variant} label={`${statusCode}`} />;
}
