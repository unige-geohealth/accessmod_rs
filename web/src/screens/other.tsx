// Sections built in later milestones, and About.
import { Callout } from "@accessmod/ui/am/messages";
import { T } from "@accessmod/ui/i18n";

export function NotYet() {
  return <Callout k="common.not_available_yet" />;
}

export function AboutScreen() {
  return (
    <div className="max-w-3xl space-y-2.5">
      <h4 className="text-lg">
        <T k="about_license" />
      </h4>
      <p>
        <T k="about_license_text" />
      </p>
      <h4 className="text-lg">
        <T k="about_acknowledge" />
      </h4>
      <p>
        <T k="about_acknowledge_text" />
      </p>
    </div>
  );
}
