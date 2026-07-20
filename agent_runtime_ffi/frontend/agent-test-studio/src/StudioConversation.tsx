import { useEffect, useRef } from 'react';
import {
  mountStudioConversation,
  type StudioConversationOptions,
} from '../../shared/studioConversation';

export function StudioConversation(options: StudioConversationOptions) {
  const hostRef = useRef<HTMLDivElement>(null);
  const optionsRef = useRef(options);
  optionsRef.current = options;

  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;
    return mountStudioConversation(host, {
      ...optionsRef.current,
      beforeSend: () => optionsRef.current.beforeSend?.(),
      onError: (message) => optionsRef.current.onError?.(message),
    });
  }, [options.token]);

  return <div className="studio-conversation-host" ref={hostRef} />;
}
