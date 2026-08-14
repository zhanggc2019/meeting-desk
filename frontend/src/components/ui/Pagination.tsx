import { ChevronLeft, ChevronRight } from "lucide-react";

interface PaginationProps {
  page: number;
  totalPages: number;
  total: number;
  disabled?: boolean;
  onPageChange: (page: number) => void;
}

/** 渲染固定页容量列表共用的上一页、下一页与总数信息。 */
export function Pagination({ page, totalPages, total, disabled = false, onPageChange }: PaginationProps) {
  return (
    <nav className="pagination" aria-label="分页">
      <span className="pagination-summary">第 {page} / {totalPages} 页，共 {total} 条</span>
      <div className="pagination-actions">
        <button className="button secondary" type="button" onClick={() => onPageChange(page - 1)} disabled={disabled || page <= 1}>
          <ChevronLeft size={15} aria-hidden="true" />上一页
        </button>
        <button className="button secondary" type="button" onClick={() => onPageChange(page + 1)} disabled={disabled || page >= totalPages}>
          下一页<ChevronRight size={15} aria-hidden="true" />
        </button>
      </div>
    </nav>
  );
}
