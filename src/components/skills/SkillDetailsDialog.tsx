import { Dialog, DialogContent, DialogHeader, DialogTitle } from "../ui/Dialog";
import { Button } from "../ui/Button";
import { Skill } from "../../types/skills";

interface SkillDetailsDialogProps {
  skill: Skill;
  isInstalled: boolean;
  onClose: () => void;
  onDownload: () => void;
  onDelete: () => void;
}

export const SkillDetailsDialog = ({
  skill,
  isInstalled,
  onClose,
  onDownload,
  onDelete,
}: SkillDetailsDialogProps) => {
  return (
    <Dialog open={true} onOpenChange={onClose}>
      <DialogContent
        className="max-w-2xl max-h-[90vh] overflow-y-auto"
        onClose={onClose}
      >
        <DialogHeader>
          <DialogTitle>{skill.title}</DialogTitle>
        </DialogHeader>

        <div className="space-y-6 mt-4">
          {/* Status Badge */}
          {isInstalled && (
            <div className="inline-flex items-center px-3 py-1 bg-green-100 dark:bg-green-900 text-green-800 dark:text-green-200 rounded-full text-sm font-medium">
              ✓ Installed
            </div>
          )}

          {/* Description */}
          <div>
            <h3 className="text-sm font-semibold text-gray-700 dark:text-gray-300 mb-2">
              Description
            </h3>
            <p className="text-gray-900 dark:text-white">{skill.description}</p>
          </div>

          {/* Author Info */}
          <div className="grid grid-cols-2 gap-4">
            <div>
              <h3 className="text-sm font-semibold text-gray-700 dark:text-gray-300 mb-2">
                Author
              </h3>
              <div className="space-y-1">
                <p className="text-gray-900 dark:text-white">
                  {skill.author || "Unknown"}
                </p>
                {skill.author_url && (
                  <a
                    href={skill.author_url}
                    target="_blank"
                    rel="noopener noreferrer"
                    className="text-sm text-blue-600 dark:text-blue-400 hover:underline"
                  >
                    Visit Author
                  </a>
                )}
                {skill.author_github && (
                  <a
                    href={`https://github.com/${skill.author_github}`}
                    target="_blank"
                    rel="noopener noreferrer"
                    className="block text-sm text-blue-600 dark:text-blue-400 hover:underline"
                  >
                    @{skill.author_github}
                  </a>
                )}
              </div>
            </div>

            <div>
              <h3 className="text-sm font-semibold text-gray-700 dark:text-gray-300 mb-2">
                License
              </h3>
              <div className="space-y-1">
                <p className="text-gray-900 dark:text-white">
                  {skill.license || "Not specified"}
                </p>
                {skill.license_url && (
                  <a
                    href={skill.license_url}
                    target="_blank"
                    rel="noopener noreferrer"
                    className="text-sm text-blue-600 dark:text-blue-400 hover:underline"
                  >
                    View License
                  </a>
                )}
              </div>
            </div>
          </div>

          {/* Tags */}
          {skill.tags.length > 0 && (
            <div>
              <h3 className="text-sm font-semibold text-gray-700 dark:text-gray-300 mb-2">
                Tags
              </h3>
              <div className="flex flex-wrap gap-2">
                {skill.tags.map((tag) => (
                  <span
                    key={tag}
                    className="px-3 py-1 text-sm bg-gray-100 dark:bg-gray-700 text-gray-700 dark:text-gray-300 rounded-full"
                  >
                    {tag}
                  </span>
                ))}
              </div>
            </div>
          )}

          {/* Source Links */}
          <div>
            <h3 className="text-sm font-semibold text-gray-700 dark:text-gray-300 mb-2">
              Source
            </h3>
            <div className="space-y-2">
              <a
                href={skill.skill_url}
                target="_blank"
                rel="noopener noreferrer"
                className="block text-sm text-blue-600 dark:text-blue-400 hover:underline"
              >
                View on GitHub →
              </a>
            </div>
          </div>

          {/* Actions */}
          <div className="flex gap-3 pt-4 border-t border-gray-200 dark:border-gray-700">
            <Button onClick={onClose} variant="outline" className="flex-1">
              Close
            </Button>
            {isInstalled ? (
              <Button
                onClick={onDelete}
                variant="outline"
                className="flex-1 text-red-600 hover:text-red-700 hover:bg-red-50 dark:hover:bg-red-900/20"
              >
                Delete
              </Button>
            ) : (
              <Button onClick={onDownload} className="flex-1">
                Download
              </Button>
            )}
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
};
