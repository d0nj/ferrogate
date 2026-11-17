import { Banner } from '@astryxdesign/core/Banner';

interface ErrorBannerProps {
  message: string | null;
}

export function ErrorBanner({ message }: ErrorBannerProps) {
  if (!message) return null;
  return <Banner status="error" title={message} />;
}
